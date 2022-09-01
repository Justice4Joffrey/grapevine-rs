/// The status of the synchronization process i.e. whether we consider
/// the associated state to be valid or not.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum SyncStatus {
    Sync,
    Syncing,
    MissedHeartbeat,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self::Syncing
    }
}
