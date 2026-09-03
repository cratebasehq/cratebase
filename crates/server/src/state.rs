use std::sync::Arc;

use cratebase_db::Db;
use cratebase_mailer::Mailer;
use cratebase_storage::Storage;

use crate::config::Config;
use crate::realtime::RealtimeHub;
use crate::request_log::RequestLogWriter;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub storage: Storage,
    pub config: Arc<Config>,
    pub realtime: RealtimeHub,
    pub mailer: Mailer,
    /// Background writer for `_request_logs`; see `request_log.rs`.
    pub request_logs: RequestLogWriter,
}
