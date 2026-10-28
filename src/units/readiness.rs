//! Waiting for configured readiness units (NetworkManager, LAN DHCP/DNS) to be active.

use crate::context::Ctx;
use crate::error::{Error, Result};
use crate::wait::{Waited, wait_until};

/// Waits until every `[readiness] units` entry is active.
pub fn wait_ready(ctx: &Ctx<'_>) -> Result<()> {
    let units = &ctx.config.readiness.units;
    let waited = wait_until(
        ctx.clock,
        ctx.log,
        "readiness",
        ctx.config.readiness.timeout_secs.get(),
        ctx.config.timing.poll_interval_secs.get(),
        || {
            let mut inactive = Vec::new();
            for unit in units {
                match ctx.host.unit_active(unit) {
                    Ok(true) => {}
                    Ok(false) => inactive.push(unit.clone()),
                    Err(e) => inactive.push(format!("{unit} ({e})")),
                }
            }
            if inactive.is_empty() {
                Ok(())
            } else {
                Err(format!("inactive: {}", inactive.join(" ")))
            }
        },
    );
    match waited {
        Waited::Met { .. } => Ok(()),
        Waited::TimedOut { last, .. } => Err(Error::Control(format!(
            "host services not ready: {last}; no changes made"
        ))),
    }
}
