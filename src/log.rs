//! Logging: sd-daemon priority prefixes, `key=value` fields, and the stage start/end guard.

use std::cell::RefCell;
use std::fmt::Display;
use std::io::Write;
use std::rc::Rc;
use std::time::Instant;

use crate::record::utc_now;

/// Log severity, mapped to sd-daemon priority prefixes for journald.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `<3>`
    Error,
    /// `<4>`
    Warning,
    /// `<6>`
    Info,
    /// `<7>`
    Debug,
}

impl Level {
    fn prefix(self) -> &'static str {
        match self {
            Self::Error => "<3>",
            Self::Warning => "<4>",
            Self::Info => "<6>",
            Self::Debug => "<7>",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Error => "error: ",
            Self::Warning => "warning: ",
            Self::Info => "",
            Self::Debug => "debug: ",
        }
    }
}

struct Inner {
    out: Box<dyn Write>,
    journal: bool,
    record: Option<Box<dyn Write>>,
}

/// Line logger. Writes to stderr (journald under systemd) and, once a run
/// record is attached, also to that record's `events` file.
pub struct Log {
    inner: RefCell<Inner>,
}

impl Log {
    /// Logs to stderr. Uses priority prefixes when stderr is the journal.
    pub fn stderr() -> Self {
        // JOURNAL_STREAM is set by systemd when stderr is connected to journald.
        let journal = std::env::var_os("JOURNAL_STREAM").is_some();
        Self::new(Box::new(std::io::stderr()), journal)
    }

    /// Logs into memory; for tests. Returns the log and a handle to read it.
    pub fn memory() -> (Self, LogBuffer) {
        let buffer = LogBuffer::default();
        (Self::new(Box::new(buffer.clone()), true), buffer)
    }

    fn new(out: Box<dyn Write>, journal: bool) -> Self {
        Self {
            inner: RefCell::new(Inner {
                out,
                journal,
                record: None,
            }),
        }
    }

    /// Also writes every following line, with a UTC timestamp, to `record`.
    pub fn attach_record(&self, record: Box<dyn Write>) {
        self.inner.borrow_mut().record = Some(record);
    }

    /// Writes one line. Write failures are ignored: logging must never stop
    /// a recovery (e.g. a closed terminal during a manual test).
    pub fn line(&self, level: Level, message: impl Display) {
        let mut inner = self.inner.borrow_mut();
        let text = message.to_string();
        let prefix = if inner.journal {
            level.prefix()
        } else {
            level.word()
        };
        let _ = writeln!(inner.out, "{prefix}{text}");
        if let Some(record) = inner.record.as_mut() {
            let _ = writeln!(record, "{} {}{text}", utc_now(), level.prefix());
            let _ = record.flush();
        }
    }

    /// Logs at error level.
    pub fn error(&self, message: impl Display) {
        self.line(Level::Error, message);
    }

    /// Logs at warning level.
    pub fn warn(&self, message: impl Display) {
        self.line(Level::Warning, message);
    }

    /// Logs at info level.
    pub fn info(&self, message: impl Display) {
        self.line(Level::Info, message);
    }

    /// Logs at debug level.
    pub fn debug(&self, message: impl Display) {
        self.line(Level::Debug, message);
    }

    /// Starts a named stage; its end line is logged when the guard is
    /// finished or dropped.
    pub fn stage<'a>(&'a self, name: &'static str) -> Stage<'a> {
        self.info(format_args!("stage start name={name}"));
        Stage {
            log: self,
            name,
            started: Instant::now(),
            outcome: None,
        }
    }
}

/// Guard that logs `stage end` with elapsed time and outcome. An unfinished
/// stage (early return or error) logs `outcome=aborted`.
pub struct Stage<'a> {
    log: &'a Log,
    name: &'static str,
    started: Instant,
    outcome: Option<String>,
}

impl Stage<'_> {
    /// Ends the stage with an outcome such as `ok` or `timeout`.
    pub fn finish(mut self, outcome: impl Display) {
        self.outcome = Some(outcome.to_string());
    }
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        let outcome = self.outcome.as_deref().unwrap_or("aborted");
        self.log.info(format_args!(
            "stage end name={} outcome={} elapsed_ms={}",
            self.name,
            outcome,
            self.started.elapsed().as_millis()
        ));
    }
}

/// In-memory log output, shared with the [`Log`] that writes it.
#[derive(Clone, Default)]
pub struct LogBuffer(Rc<RefCell<Vec<u8>>>);

impl LogBuffer {
    /// Everything logged so far.
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.borrow()).into_owned()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Quotes a value for a `key=value` field, escaping quotes and control
/// characters so untrusted phone output stays on one line.
pub fn quote(value: &str) -> String {
    format!("\"{}\"", value.escape_debug())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_lines_have_priority_prefix() {
        let (log, buffer) = Log::memory();
        log.error("boom");
        log.info("fine");
        assert_eq!(buffer.contents(), "<3>boom\n<6>fine\n");
    }

    #[test]
    fn stage_logs_end_even_when_dropped() {
        let (log, buffer) = Log::memory();
        {
            let _stage = log.stage("phone");
        }
        log.stage("usb").finish("ok");
        let text = buffer.contents();
        assert!(text.contains("stage end name=phone outcome=aborted"));
        assert!(text.contains("stage end name=usb outcome=ok"));
    }

    #[test]
    fn quote_keeps_untrusted_text_on_one_line() {
        assert_eq!(quote("rndis,adb\r\n"), "\"rndis,adb\\r\\n\"");
        assert_eq!(quote("a \"b\""), "\"a \\\"b\\\"\"");
    }
}
