//! The realtime publish seam.
//!
//! The SSE service itself is W4b-2. What already has to exist is the
//! *call site*: every committed record mutation must announce itself
//! exactly once, after the transaction commits and with the record in the
//! shape a subscriber would have read it. Putting that seam in now means
//! W4b-2 fills in one function body instead of threading a new call
//! through five handlers and the batch service.
//!
//! Contract for the implementation:
//!
//! * Called **after** `App::run_in_transaction` returned `Ok`, never from
//!   inside the transaction — a subscriber that reacts by reading the
//!   record must find it committed.
//! * The `record` is the post-write state (for a delete, the row as it
//!   was), already enriched but **not** serialized: per-subscriber
//!   `fields`/`expand` are the realtime service's business.
//! * Must not block the request: W4b-2 spawns the fan-out.

use std::sync::Arc;

use cratebase_core::{Collection, Record};

pub use cratebase_core::RecordAction;

/// Announce a committed record mutation to realtime subscribers.
///
// W4b-2: implement — resolve the `<collection>` and `<collection>/<id>`
// topics, evaluate each subscriber's `listRule`/`viewRule` and `?filter=`
// in-process against `record`, and write one SSE frame per match through
// `on_realtime_message_send`.
pub fn publish(
    app: &crate::app::App,
    collection: &Arc<Collection>,
    action: RecordAction,
    record: &Record,
) {
    // Deliberately cheap and total: until W4b-2 lands this is a trace, not
    // a no-op that silently swallows a wiring mistake.
    let _ = app;
    tracing::trace!(
        collection = %collection.name,
        record = %record.id(),
        action = action.as_str(),
        "realtime publish (W4b-2 pending)"
    );
}
