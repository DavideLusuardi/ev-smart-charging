//! The stations connected right now, one entry per station identity.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::oneshot;

use crate::station_id::StationId;

/// Connected stations.
///
/// A station that reconnects under the same identity replaces its old connection: the old
/// socket is usually half-dead (the station saw the drop before the CSMS did), so the old
/// connection is told to stop.
#[derive(Debug, Default)]
pub struct Registry {
    entries: RwLock<HashMap<StationId, Entry>>,
    next_generation: AtomicU64,
}

#[derive(Debug)]
struct Entry {
    generation: u64,
    /// Dropping the entry drops this sender, which stops the connection.
    _stop: oneshot::Sender<()>,
}

/// What a connection gets back when it registers.
#[derive(Debug)]
pub(crate) struct Registration {
    /// Tells this connection apart from earlier and later ones of the same station.
    pub generation: u64,
    /// Resolves once the connection has been replaced by a newer one.
    pub stop: oneshot::Receiver<()>,
}

impl Registry {
    /// Registers a new connection for `id`, stopping any previous one.
    pub(crate) fn register(&self, id: StationId) -> Registration {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop) = oneshot::channel();
        let entry = Entry {
            generation,
            _stop: stop_tx,
        };
        let previous = self.write().insert(id, entry);
        drop(previous);
        Registration { generation, stop }
    }

    /// Removes `id` if it still belongs to the connection with `generation`.
    pub(crate) fn unregister(&self, id: &StationId, generation: u64) {
        let mut entries = self.write();
        if entries.get(id).is_some_and(|e| e.generation == generation) {
            entries.remove(id);
        }
    }

    /// Whether a station is connected.
    pub fn is_connected(&self, id: &StationId) -> bool {
        self.read().contains_key(id)
    }

    /// The connected stations, sorted by identity.
    pub fn connected(&self) -> Vec<StationId> {
        let mut ids: Vec<_> = self.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<StationId, Entry>> {
        self.entries.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<StationId, Entry>> {
        self.entries.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot::error::TryRecvError;

    fn id(s: &str) -> StationId {
        StationId::try_from(s).unwrap()
    }

    #[test]
    fn reconnect_stops_the_old_connection() {
        let registry = Registry::default();
        let mut first = registry.register(id("CS1"));
        assert_eq!(first.stop.try_recv(), Err(TryRecvError::Empty));

        let second = registry.register(id("CS1"));
        assert_eq!(first.stop.try_recv(), Err(TryRecvError::Closed));

        // The old connection cleaning up must not remove the new one.
        registry.unregister(&id("CS1"), first.generation);
        assert!(registry.is_connected(&id("CS1")));
        registry.unregister(&id("CS1"), second.generation);
        assert!(!registry.is_connected(&id("CS1")));
    }

    #[test]
    fn lists_connected_stations_sorted() {
        let registry = Registry::default();
        let _b = registry.register(id("B"));
        let _a = registry.register(id("A"));
        assert_eq!(registry.connected(), vec![id("A"), id("B")]);
    }
}
