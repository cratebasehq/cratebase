//! The contract between the JavaScript runtime and the application that
//! embeds it.
//!
//! The runtime never touches a database, a mailer or an HTTP client
//! itself: every `$app.*`, `$http.send`, `routerAdd`, `cronAdd` and
//! `on*` call in JavaScript ends up as a method on [`HostApi`], which
//! the server crate implements on top of its `App`. Keeping the runtime
//! behind this trait means it can be built, tested and reasoned about in
//! isolation with a mock host.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cratebase_core::{AppError, Collection, Record, Settings};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Identifies a JavaScript hook handler registered through one of the
/// `on*` globals. Ids are deterministic (`<file>:<hook>:<tags>:<ordinal>`)
/// so every worker in the pool derives the same id for the same handler
/// and any worker can execute it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HookHandlerId(pub String);

/// Identifies a JavaScript route handler registered through `routerAdd`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RouteHandlerId(pub String);

/// Identifies a JavaScript cron job registered through `cronAdd`. This is
/// the user-supplied job id (`cronAdd("cleanup", ...)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CronHandlerId(pub String);

impl std::fmt::Display for HookHandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::fmt::Display for RouteHandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::fmt::Display for CronHandlerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! hook_kinds {
    ($( $variant:ident => $js:literal ),* $(,)?) => {
        /// Every hook the server exposes (spec section 7.1), named after
        /// PocketBase's `on*` registration functions.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum HookKind {
            $( $variant, )*
        }

        impl HookKind {
            /// All hook kinds, in declaration order.
            pub const ALL: &'static [HookKind] = &[ $( HookKind::$variant, )* ];

            /// The JavaScript registration function (`onRecordCreate`, ...).
            pub fn js_name(self) -> &'static str {
                match self {
                    $( HookKind::$variant => $js, )*
                }
            }

            /// Parse a JavaScript registration function name.
            pub fn from_js_name(name: &str) -> Option<HookKind> {
                match name {
                    $( $js => Some(HookKind::$variant), )*
                    _ => None,
                }
            }
        }
    };
}

hook_kinds! {
    Bootstrap => "onBootstrap",
    Serve => "onServe",
    Terminate => "onTerminate",

    RecordEnrich => "onRecordEnrich",
    RecordValidate => "onRecordValidate",

    RecordCreate => "onRecordCreate",
    RecordCreateExecute => "onRecordCreateExecute",
    RecordAfterCreateSuccess => "onRecordAfterCreateSuccess",
    RecordAfterCreateError => "onRecordAfterCreateError",
    RecordUpdate => "onRecordUpdate",
    RecordUpdateExecute => "onRecordUpdateExecute",
    RecordAfterUpdateSuccess => "onRecordAfterUpdateSuccess",
    RecordAfterUpdateError => "onRecordAfterUpdateError",
    RecordDelete => "onRecordDelete",
    RecordDeleteExecute => "onRecordDeleteExecute",
    RecordAfterDeleteSuccess => "onRecordAfterDeleteSuccess",
    RecordAfterDeleteError => "onRecordAfterDeleteError",

    RecordCreateRequest => "onRecordCreateRequest",
    RecordUpdateRequest => "onRecordUpdateRequest",
    RecordDeleteRequest => "onRecordDeleteRequest",
    RecordListRequest => "onRecordListRequest",
    RecordViewRequest => "onRecordViewRequest",

    CollectionValidate => "onCollectionValidate",
    CollectionCreate => "onCollectionCreate",
    CollectionCreateExecute => "onCollectionCreateExecute",
    CollectionAfterCreateSuccess => "onCollectionAfterCreateSuccess",
    CollectionAfterCreateError => "onCollectionAfterCreateError",
    CollectionUpdate => "onCollectionUpdate",
    CollectionUpdateExecute => "onCollectionUpdateExecute",
    CollectionAfterUpdateSuccess => "onCollectionAfterUpdateSuccess",
    CollectionAfterUpdateError => "onCollectionAfterUpdateError",
    CollectionDelete => "onCollectionDelete",
    CollectionDeleteExecute => "onCollectionDeleteExecute",
    CollectionAfterDeleteSuccess => "onCollectionAfterDeleteSuccess",
    CollectionAfterDeleteError => "onCollectionAfterDeleteError",
    CollectionCreateRequest => "onCollectionCreateRequest",
    CollectionUpdateRequest => "onCollectionUpdateRequest",
    CollectionDeleteRequest => "onCollectionDeleteRequest",
    CollectionsListRequest => "onCollectionsListRequest",
    CollectionViewRequest => "onCollectionViewRequest",
    CollectionsImportRequest => "onCollectionsImportRequest",

    RecordAuthRequest => "onRecordAuthRequest",
    RecordAuthWithPasswordRequest => "onRecordAuthWithPasswordRequest",
    RecordAuthWithOAuth2Request => "onRecordAuthWithOAuth2Request",
    RecordAuthWithOTPRequest => "onRecordAuthWithOTPRequest",
    RecordAuthRefreshRequest => "onRecordAuthRefreshRequest",
    RecordRequestOTPRequest => "onRecordRequestOTPRequest",
    RecordRequestPasswordResetRequest => "onRecordRequestPasswordResetRequest",
    RecordConfirmPasswordResetRequest => "onRecordConfirmPasswordResetRequest",
    RecordRequestVerificationRequest => "onRecordRequestVerificationRequest",
    RecordConfirmVerificationRequest => "onRecordConfirmVerificationRequest",
    RecordRequestEmailChangeRequest => "onRecordRequestEmailChangeRequest",
    RecordConfirmEmailChangeRequest => "onRecordConfirmEmailChangeRequest",

    MailerSend => "onMailerSend",
    MailerRecordVerificationSend => "onMailerRecordVerificationSend",
    MailerRecordPasswordResetSend => "onMailerRecordPasswordResetSend",
    MailerRecordEmailChangeSend => "onMailerRecordEmailChangeSend",
    MailerRecordOTPSend => "onMailerRecordOTPSend",
    MailerRecordAuthAlertSend => "onMailerRecordAuthAlertSend",

    RealtimeConnectRequest => "onRealtimeConnectRequest",
    RealtimeSubscribeRequest => "onRealtimeSubscribeRequest",
    RealtimeMessageSend => "onRealtimeMessageSend",

    FileDownloadRequest => "onFileDownloadRequest",
    FileTokenRequest => "onFileTokenRequest",

    BackupCreate => "onBackupCreate",
    BackupRestore => "onBackupRestore",

    SettingsListRequest => "onSettingsListRequest",
    SettingsUpdateRequest => "onSettingsUpdateRequest",
    SettingsReload => "onSettingsReload",

    BatchRequest => "onBatchRequest",
}

/// An outbound HTTP request issued by `$http.send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<Vec<u8>>,
    /// `0` means the host's default.
    #[serde(default)]
    pub timeout_secs: u64,
}

/// The response the host produced for an [`HttpRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Vec<u8>,
}

/// The kind of single-use record token `$tokens` / `$mails` need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordTokenKind {
    Auth,
    Verification,
    PasswordReset,
    EmailChange,
    File,
}

/// A closure executed inside a host transaction. It receives a host bound
/// to the transaction so every call joins it.
pub type TransactionFn =
    Box<dyn FnOnce(Arc<dyn HostApi>) -> BoxFuture<'static, Result<(), AppError>> + Send>;

/// Everything the JavaScript globals need from the embedding application.
///
/// All record/collection methods take and return `cratebase_core` domain
/// types; the runtime handles the JSON conversion on both sides. Methods
/// with a default implementation are optional extension points the server
/// fills in when the corresponding subsystem exists.
#[async_trait]
pub trait HostApi: Send + Sync + 'static {
    /// The current application settings (`$app.settings()`).
    fn settings(&self) -> Settings;

    async fn find_collection(&self, name_or_id: &str) -> Result<Arc<Collection>, AppError>;

    async fn find_record_by_id(&self, collection: &str, id: &str) -> Result<Record, AppError>;

    /// `filter` uses the PocketBase filter language with `{:name}`
    /// placeholders bound from `params`. `limit <= 0` means no limit.
    async fn find_records_by_filter(
        &self,
        collection: &str,
        filter: &str,
        sort: &str,
        limit: i64,
        offset: i64,
        params: Map<String, Value>,
    ) -> Result<Vec<Record>, AppError>;

    /// A read-only escape hatch for hook authors who need SQL a filter
    /// string cannot express (`GROUP BY`, window functions, joins): run
    /// `sql` (must be a `SELECT`/`WITH` statement) and hand back plain
    /// rows with no `Record` hydration, matching PocketBase's `$app.db()`
    /// use case at a fraction of its API surface. `{:name}` placeholders
    /// in `sql` are bound to real, driver-level parameters from `params`
    /// (never string-substituted), so this carries no more injection risk
    /// than a hand-written parameterized query.
    async fn raw_query(
        &self,
        sql: &str,
        params: Map<String, Value>,
    ) -> Result<Vec<Map<String, Value>>, AppError>;

    /// Create (`record.is_new()`) or update a record, returning the
    /// persisted state.
    async fn save_record(&self, record: Record) -> Result<Record, AppError>;

    async fn delete_record(&self, record: Record) -> Result<(), AppError>;

    async fn save_collection(&self, collection: Collection) -> Result<Collection, AppError>;

    async fn delete_collection(&self, name_or_id: &str) -> Result<(), AppError>;

    /// Run `f` inside a transaction. The host must invoke `f` *inline*
    /// (awaiting the returned future in its own task) rather than
    /// spawning it, because the runtime coordinates the JavaScript side of
    /// the transaction with the worker thread through that future.
    async fn run_in_transaction(&self, f: TransactionFn) -> Result<(), AppError>;

    /// `to` is a list of `(address, name)` pairs.
    async fn send_mail(
        &self,
        to: Vec<(String, String)>,
        subject: String,
        html: String,
    ) -> Result<(), AppError>;

    async fn http_send(&self, req: HttpRequest) -> Result<HttpResponse, AppError>;

    /// `level` follows PocketBase's slog levels: -4 debug, 0 info,
    /// 4 warn, 8 error.
    fn log(&self, level: i32, message: &str, data: Value);

    fn store_get(&self, key: &str) -> Option<Value>;
    fn store_set(&self, key: &str, value: Value);

    /// Called once per `routerAdd` with the HTTP method (upper-case) and
    /// the PocketBase-style path pattern (`/hello/{name}`, `/files/{path...}`).
    /// The server wires the axum route to [`crate::Runtime::call_route`]
    /// with `handler`.
    fn register_route(&self, method: &str, path: &str, handler: RouteHandlerId);

    fn register_cron(&self, id: &str, expr: &str, handler: CronHandlerId);
    fn remove_cron(&self, id: &str);

    /// Register a JavaScript hook. The server binds a Rust handler on the
    /// matching `Hook<E>` that snapshots the event, calls
    /// [`crate::Runtime::call_hook`] and applies the outcome (see
    /// [`crate::JsEventOutcome`]).
    fn register_hook(
        &self,
        hook: HookKind,
        tags: Vec<String>,
        handler: HookHandlerId,
        priority: i32,
    );
    fn unregister_hook(&self, id: &str);

    /// Issue a signed record token (`$tokens.*`, `$mails.*`). Needs the
    /// record's `tokenKey`, which lives in the auth subsystem, so the
    /// default says "not available yet".
    async fn create_record_token(
        &self,
        _record: &Record,
        _kind: RecordTokenKind,
    ) -> Result<String, AppError> {
        Err(AppError::internal(
            "record tokens are not available in this runtime yet",
        ))
    }

    /// Verify a plaintext password against a record's stored hash
    /// (`record.validatePassword`). Default: not available.
    async fn validate_password(&self, _record: &Record, _password: &str) -> Result<bool, AppError> {
        Err(AppError::internal(
            "password validation is not available in this runtime yet",
        ))
    }

    /// Hash a plaintext password for `record.setPassword`. The default
    /// stores the plaintext under a marker the server can recognise
    /// (`{"__plainPassword": ...}`) so the write path hashes it; hosts
    /// with an auth subsystem override this.
    fn hash_password(&self, password: &str) -> Value {
        serde_json::json!({ "__plainPassword": password })
    }
}
