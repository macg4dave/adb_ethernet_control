//! The `adb-wan` system user: lookup from `/etc/passwd`, and the effective UID check.

use std::path::PathBuf;

/// Name of the service user and group.
pub const USER: &str = "adb-wan";

/// A user's IDs and home, from `/etc/passwd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// User ID.
    pub uid: u32,
    /// Primary group ID.
    pub gid: u32,
    /// Home directory.
    pub home: PathBuf,
}

/// Looks up `name` in passwd-format text.
pub fn parse_passwd(text: &str, name: &str) -> Option<Account> {
    text.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 7 || fields[0] != name {
            return None;
        }
        Some(Account {
            uid: fields[2].parse().ok()?,
            gid: fields[3].parse().ok()?,
            home: PathBuf::from(fields[5]),
        })
    })
}

/// Looks up a user in `/etc/passwd`.
pub fn lookup(name: &str) -> Option<Account> {
    let text = std::fs::read_to_string("/etc/passwd").ok()?;
    parse_passwd(&text, name)
}

/// Effective UID of this process, from `/proc/self/status`.
pub fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

/// Whether this process runs as root.
pub fn is_root() -> bool {
    effective_uid() == Some(0)
}

/// When running as root, the IDs to drop the ADB server to, so it uses the
/// service user's authorized key; `None` otherwise.
pub fn adb_server_identity() -> Option<(u32, u32)> {
    if is_root() {
        lookup(USER).map(|a| (a.uid, a.gid))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_user() {
        let text = "root:x:0:0:root:/root:/bin/bash\nadb-wan:x:985:980:ADB WAN recovery:/var/lib/adb-wan:/usr/sbin/nologin\n";
        assert_eq!(
            parse_passwd(text, "adb-wan"),
            Some(Account {
                uid: 985,
                gid: 980,
                home: "/var/lib/adb-wan".into()
            })
        );
        assert_eq!(parse_passwd(text, "nobody"), None);
        assert_eq!(parse_passwd("adb-wan:x:bad:1:::\n", "adb-wan"), None);
    }
}
