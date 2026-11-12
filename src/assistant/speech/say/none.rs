//! No speech program on this OS.

use std::path::Path;

use super::super::voices::Voice;
use super::{Job, Plan};

pub fn plan(_job: &Job, _file: &Path) -> Plan {
    Plan::Audio(None)
}

pub fn voices() -> Vec<Voice> {
    Vec::new()
}
