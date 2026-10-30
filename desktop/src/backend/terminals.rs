//! Live agent and shell terminals. Callers ask here whether a chat has a live
//! terminal instead of reading daemon inventory rows themselves.
use super::terminal::OpenOutcome;
use super::terminal_daemon::Host;
use super::{sid, text, Error, Result, Value};
use std::{collections::BTreeSet, ops::RangeInclusive, path::Path};

/// The UI clamps every terminal view to these sizes.
const COLUMNS: RangeInclusive<u64> = 10..=500;
const ROWS: RangeInclusive<u64> = 2..=200;

pub struct Terminals {
    host: Host,
}

/// One validated inventory read. Every query on it sees the same rows.
pub struct Inventory {
    rows: Vec<Value>,
}

impl Inventory {
    fn new(rows: Vec<Value>) -> Result<Self> {
        let mut ids = BTreeSet::new();
        for row in &rows {
            let generation = text(row, "generation");
            if !ids.insert(text(row, "recordId"))
                || sid(generation).ok().as_deref() != Some(generation)
            {
                return Err(Error::new("Terminal inventory response is invalid."));
            }
        }
        Ok(Self { rows })
    }
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }
}

impl Terminals {
    pub const fn new(host: Host) -> Self {
        Self { host }
    }
    pub fn inventory(&self) -> Result<Inventory> {
        Inventory::new(self.host.inventory()?)
    }
    pub fn open(
        &self,
        record: &Value,
        packet: &str,
        new: bool,
        root: &Path,
    ) -> Result<OpenOutcome> {
        self.host.open(record, packet, new, root)
    }
    pub fn open_shell(&self, record: &Value) -> Result<Value> {
        self.host.open_shell(record)
    }
    pub fn open_demo(&self, record: &Value) -> Result<()> {
        self.host.open_demo(record)
    }
    pub fn send(&self, id: &str, data: &str) -> Result<()> {
        self.host.input(id, data)
    }
    pub fn poll(&self, id: &str, cursor: u64, generation: &str) -> Result<Value> {
        self.host.poll(id, cursor, generation)
    }
    /// Resize to the `cols` and `rows` of `size`.
    pub fn resize(&self, id: &str, size: &Value) -> Result<()> {
        let bounded = |key: &str, range: RangeInclusive<u64>, message: &str| {
            size[key]
                .as_u64()
                .filter(|n| range.contains(n))
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| Error::new(message))
        };
        self.host.resize(
            id,
            bounded("cols", COLUMNS, "Invalid columns.")?,
            bounded("rows", ROWS, "Invalid rows.")?,
        )
    }
    pub fn close(&self, id: &str, generation: &str) -> Result<()> {
        self.host.close(id, generation)
    }
    #[cfg(test)]
    pub(super) const fn host(&self) -> &Host {
        &self.host
    }
}

#[cfg(test)]
mod tests;
