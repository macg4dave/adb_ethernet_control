//! Our own loopback ADB server, run as a child and stopped on every exit path.

use std::cell::RefCell;
use std::process::Child;
use std::time::{Duration, Instant};

use super::Adb;
use super::device::TransportId;
use crate::config;
use crate::error::{Error, Result};
use crate::process::{Cmd, CommandOutput, run, spawn};

/// How often a starting server is checked.
const START_POLL: Duration = Duration::from_millis(250);

/// The real ADB: a server child on `127.0.0.1:<port>` and clients pinned to
/// it with `-H`/`-P`, so no client ever auto-spawns a second server.
pub struct RealAdb {
    config: config::Adb,
    run_as: Option<(u32, u32)>,
    child: RefCell<Option<Child>>,
}

impl RealAdb {
    /// An ADB client/server pair. `run_as` drops the server to that uid/gid
    /// (used when invoked as root), so it uses the service user's key.
    pub fn new(config: &config::Adb, run_as: Option<(u32, u32)>) -> Self {
        Self {
            config: config.clone(),
            run_as,
            child: RefCell::new(None),
        }
    }

    fn client(&self) -> Cmd {
        Cmd::new(&self.config.binary, self.config.command_timeout_secs.get())
            .args(["-H", "127.0.0.1", "-P"])
            .arg(self.config.server_port.to_string())
            .env_remove("ADB_SERVER_SOCKET")
            .env_remove("ANDROID_ADB_SERVER_PORT")
    }
}

impl Adb for RealAdb {
    fn start_server(&self) -> Result<()> {
        if self.child.borrow().is_some() {
            return Ok(());
        }
        let home = self.config.home.display().to_string();
        let mut server = Cmd::new(&self.config.binary, Duration::ZERO)
            .args(["-L"])
            .arg(format!("tcp:localhost:{}", self.config.server_port))
            .args(["server", "nodaemon"])
            .env("HOME", home)
            .env_remove("ADB_SERVER_SOCKET")
            .env_remove("ANDROID_ADB_SERVER_PORT")
            .run_as(self.run_as);
        if self.config.libusb {
            server = server.env("ADB_LIBUSB", "1");
        }
        let mut child = spawn(&server)?;
        let deadline = Instant::now() + self.config.server_start_timeout_secs.get();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(Error::Control(format!(
                    "ADB server exited during startup ({status}); is port {} already in use?",
                    self.config.server_port
                )));
            }
            if run(&self.client().arg("devices")).is_ok_and(|o| o.success()) {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Control(format!(
                    "ADB server did not answer within {}s",
                    self.config.server_start_timeout_secs.get().as_secs()
                )));
            }
            std::thread::sleep(START_POLL);
        }
        *self.child.borrow_mut() = Some(child);
        Ok(())
    }

    fn devices(&self) -> Result<String> {
        let output = run(&self.client().args(["devices", "-l"]))?;
        if output.success() {
            Ok(output.stdout)
        } else {
            Err(Error::Control(format!(
                "adb devices failed: {}",
                output.summary()
            )))
        }
    }

    fn shell(&self, transport: TransportId, args: &[&str]) -> Result<CommandOutput> {
        let cmd = self
            .client()
            .args(["-t", &transport.to_string(), "shell", "-T", "-n"])
            .args(args.iter().copied());
        run(&cmd)
    }

    fn stop_server(&self) {
        if let Some(mut child) = self.child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RealAdb {
    fn drop(&mut self) {
        self.stop_server();
    }
}
