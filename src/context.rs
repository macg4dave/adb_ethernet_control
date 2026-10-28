//! Everything a flow needs, passed in so tests can swap the real system for fakes.

use crate::adb::Adb;
use crate::config::Config;
use crate::host::Host;
use crate::log::Log;
use crate::recovery::marker::Markers;
use crate::wait::Clock;

/// Dependencies of a flow.
pub struct Ctx<'a> {
    /// Loaded settings.
    pub config: &'a Config,
    /// Phone access.
    pub adb: &'a dyn Adb,
    /// Host observation.
    pub host: &'a dyn Host,
    /// Time source.
    pub clock: &'a dyn Clock,
    /// Logger.
    pub log: &'a Log,
    /// Runtime markers.
    pub markers: &'a Markers,
}
