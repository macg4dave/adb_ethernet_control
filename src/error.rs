//! Crate error type and the single mapping to exit codes 0 / 1 / 2.

use std::process::ExitCode;

/// Everything that can stop a command. All of these exit with code 1.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The config file is missing, unreadable or invalid.
    #[error("config: {0}")]
    Config(String),
    /// An external program did not finish within its timeout.
    #[error("{program} timed out after {secs}s")]
    Timeout {
        /// Program and arguments, for the log.
        program: String,
        /// The timeout that expired.
        secs: u64,
    },
    /// An external program could not be started.
    #[error("cannot run {program}: {source}")]
    Spawn {
        /// Program that failed to start.
        program: String,
        /// Underlying OS error.
        source: std::io::Error,
    },
    /// A file or directory operation failed.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Underlying OS error.
        source: std::io::Error,
    },
    /// Readiness, phone access or another precondition failed; nothing more
    /// was changed.
    #[error("{0}")]
    Control(String),
}

impl Error {
    /// Wraps an I/O error with a description of what was attempted.
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

/// Crate result type.
pub type Result<T> = std::result::Result<T, Error>;

/// How a command that ran to completion ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// WAN healthy or restored, or a read-only command completed.
    Success,
    /// WAN still down on the final check, or a report/test did not pass.
    Failed,
}

impl Exit {
    /// The documented process exit code.
    pub fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failed => 2,
        }
    }
}

/// Maps a command result to the process exit code: 0, 1 (error) or 2.
pub fn exit_code(result: &Result<Exit>) -> ExitCode {
    match result {
        Ok(exit) => ExitCode::from(exit.code()),
        Err(_) => ExitCode::from(1),
    }
}
