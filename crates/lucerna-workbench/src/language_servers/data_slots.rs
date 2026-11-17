//! Numbers a language server's `{data_dir}` per window.
//!
//! Two windows on one root run two processes of one server (a
//! [`LanguageServers`](super::LanguageServers) per window). Servers such as
//! jdtls lock their data directory, so each process takes the smallest
//! slot number no other window holds for the same [`ServerKey`], and
//! `data_dir_for` turns it into a directory of its own. Slot 0 is the
//! directory every single-window session has always used.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use gpui::Global;
use lucerna_lsp::ServerKey;

/// The slot numbers in use, per key, across every window.
#[derive(Clone, Default)]
pub(crate) struct DataDirSlots(Arc<Mutex<HashMap<ServerKey, BTreeSet<u32>>>>);

impl Global for DataDirSlots {}

impl DataDirSlots {
    fn lock(&self) -> MutexGuard<'_, HashMap<ServerKey, BTreeSet<u32>>> {
        // A panic while holding the lock leaves a consistent map.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the smallest number `key` has not got in use. It is free
    /// again when the returned guard drops.
    pub(crate) fn claim(&self, key: &ServerKey) -> DataDirSlot {
        let mut map = self.lock();
        let used = map.entry(key.clone()).or_default();
        let number = (0..)
            .find(|n| !used.contains(n))
            .expect("fewer than 2^32 slots");
        used.insert(number);
        DataDirSlot {
            slots: self.clone(),
            key: key.clone(),
            number,
        }
    }
}

/// A claimed slot; dropping it gives the number back.
pub(crate) struct DataDirSlot {
    slots: DataDirSlots,
    key: ServerKey,
    number: u32,
}

impl DataDirSlot {
    pub(crate) fn number(&self) -> u32 {
        self.number
    }
}

impl Drop for DataDirSlot {
    fn drop(&mut self) {
        let mut map = self.slots.lock();
        if let Some(used) = map.get_mut(&self.key) {
            used.remove(&self.number);
            if used.is_empty() {
                map.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn key(root: &str) -> ServerKey {
        ServerKey {
            root: Arc::from(Path::new(root)),
            server: "jdtls".into(),
        }
    }

    #[test]
    fn claims_the_smallest_free_slot() {
        let slots = DataDirSlots::default();
        let a = key("/ws/a");
        let first = slots.claim(&a);
        let second = slots.claim(&a);
        assert_eq!((first.number(), second.number()), (0, 1));
        drop(first);
        let again = slots.claim(&a);
        assert_eq!(again.number(), 0);
        assert_eq!(slots.claim(&a).number(), 2);
        assert_eq!(slots.claim(&key("/ws/b")).number(), 0);
    }

    #[test]
    fn dropping_the_last_slot_forgets_the_key() {
        let slots = DataDirSlots::default();
        drop(slots.claim(&key("/ws/a")));
        assert!(slots.lock().is_empty());
    }
}
