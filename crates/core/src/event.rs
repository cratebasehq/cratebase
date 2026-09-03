//! Events emitted by the storage layer so higher layers (realtime,
//! hooks, webhooks, the JS runtime) can react without `db` depending on
//! them.

use std::sync::Arc;

use crate::collection::Collection;
use crate::record::Record;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordAction {
    Create,
    Update,
    Delete,
}

impl RecordAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RecordAction::Create => "create",
            RecordAction::Update => "update",
            RecordAction::Delete => "delete",
        }
    }
}

/// A committed record mutation.
#[derive(Debug, Clone)]
pub struct RecordChanged {
    pub action: RecordAction,
    pub collection: Arc<Collection>,
    pub record: Record,
    /// The row as it was before an update or delete.
    pub previous: Option<Record>,
}
