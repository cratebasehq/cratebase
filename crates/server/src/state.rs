use std::sync::Arc;

use cratebase_db::Db;
use cratebase_mailer::Mailer;
use cratebase_storage::Storage;

use crate::config::Config;
use crate::realtime::RealtimeHub;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub storage: Storage,
    pub config: Arc<Config>,
    pub realtime: RealtimeHub,
    pub mailer: Mailer,
}
