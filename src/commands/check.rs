//! `check`: read-only WAN health check with exit code, no changes.

use crate::context::Ctx;
use crate::error::{Exit, Result};
use crate::recovery::cycle::check_wan;

/// Checks WAN once: exit 0 when up, 2 when down. Never starts ADB.
pub fn run(ctx: &Ctx<'_>) -> Result<Exit> {
    if check_wan(ctx, "check").is_up() {
        Ok(Exit::Success)
    } else {
        Ok(Exit::Failed)
    }
}
