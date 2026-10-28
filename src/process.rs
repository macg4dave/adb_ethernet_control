//! The only place that runs external programs: fixed arguments, timeouts, capped untrusted output.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::log::quote;

/// Output kept per stream; the rest is discarded.
const MAX_OUTPUT: usize = 64 * 1024;
/// Output shown in log summaries.
const SUMMARY_CHARS: usize = 300;
/// How often a running child is checked for exit.
const CHILD_POLL: Duration = Duration::from_millis(10);

/// A program invocation with fixed arguments and a mandatory timeout.
#[derive(Debug, Clone)]
pub struct Cmd {
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    timeout: Duration,
    run_as: Option<(u32, u32)>,
}

impl Cmd {
    /// A command for `program` that is killed after `timeout`.
    pub fn new(program: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            timeout,
            run_as: None,
        }
    }

    /// Appends one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable for the child.
    #[must_use]
    pub fn env(mut self, key: &str, value: impl Into<String>) -> Self {
        self.env.push((key.to_owned(), value.into()));
        self
    }

    /// Removes an inherited environment variable from the child.
    #[must_use]
    pub fn env_remove(mut self, key: &str) -> Self {
        self.env_remove.push(key.to_owned());
        self
    }

    /// Runs the child as this uid/gid (requires root). Supplementary groups
    /// are dropped.
    #[must_use]
    pub fn run_as(mut self, ids: Option<(u32, u32)>) -> Self {
        self.run_as = ids;
        self
    }

    /// Program and arguments as one line, for logs.
    pub fn display(&self) -> String {
        let mut text = self.program.display().to_string();
        for arg in &self.args {
            text.push(' ');
            text.push_str(arg);
        }
        text
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        for key in &self.env_remove {
            command.env_remove(key);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        if let Some((uid, gid)) = self.run_as {
            command.uid(uid).gid(gid);
        }
        command
    }

    fn spawn_error(&self, source: std::io::Error) -> Error {
        Error::Spawn {
            program: self.display(),
            source,
        }
    }
}

/// Captured result of a finished command. Output is untrusted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit code; `None` if killed by a signal.
    pub code: Option<i32>,
    /// Standard output, lossily decoded and capped.
    pub stdout: String,
    /// Standard error, lossily decoded and capped.
    pub stderr: String,
}

impl CommandOutput {
    /// A successful result with the given stdout; convenient for fakes.
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    /// A failed result with the given exit code and stderr.
    pub fn failed(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    /// Whether the program exited with status 0.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// `exit=N stdout="..." stderr="..."`, truncated, for logs.
    pub fn summary(&self) -> String {
        let code = self
            .code
            .map_or_else(|| "signal".to_owned(), |c| c.to_string());
        format!(
            "exit={code} stdout={} stderr={}",
            quote(&truncate(self.stdout.trim(), SUMMARY_CHARS)),
            quote(&truncate(self.stderr.trim(), SUMMARY_CHARS))
        )
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max).collect();
    short.push_str("...");
    short
}

/// Runs `cmd` to completion or kills it at its timeout.
pub fn run(cmd: &Cmd) -> Result<CommandOutput> {
    let mut child = cmd
        .command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| cmd.spawn_error(e))?;
    let stdout = reader(child.stdout.take());
    let stderr = reader(child.stderr.take());
    let deadline = Instant::now() + cmd.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout {
                    program: cmd.display(),
                    secs: cmd.timeout.as_secs(),
                });
            }
            Ok(None) => thread::sleep(CHILD_POLL),
            Err(e) => return Err(Error::io(format!("waiting for {}", cmd.display()), e)),
        }
    };
    Ok(CommandOutput {
        code: status.code(),
        stdout: collect(&stdout),
        stderr: collect(&stderr),
    })
}

/// Starts a long-running child (the ADB server) with null stdin/stdout and
/// inherited stderr, so its messages reach the journal.
pub fn spawn(cmd: &Cmd) -> Result<Child> {
    cmd.command()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| cmd.spawn_error(e))
}

fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut kept = Vec::new();
            let mut chunk = [0_u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let room = MAX_OUTPUT.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
            let _ = sender.send(kept);
        });
    }
    receiver
}

fn collect(receiver: &mpsc::Receiver<Vec<u8>>) -> String {
    // A grandchild that inherited the pipe could keep it open; don't wait
    // for it beyond a short grace period.
    receiver
        .recv_timeout(Duration::from_millis(500))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_is_quoted_and_truncated() {
        let output = CommandOutput {
            code: Some(1),
            stdout: "x".repeat(400),
            stderr: "bad\nthing".into(),
        };
        let summary = output.summary();
        assert!(summary.starts_with("exit=1 stdout=\"xxx"));
        assert!(summary.contains("...\""));
        assert!(summary.ends_with("stderr=\"bad\\nthing\""));
    }

    #[test]
    fn display_joins_program_and_args() {
        let cmd = Cmd::new("/usr/bin/adb", Duration::from_secs(1)).args(["devices", "-l"]);
        assert_eq!(cmd.display(), "/usr/bin/adb devices -l");
    }
}
