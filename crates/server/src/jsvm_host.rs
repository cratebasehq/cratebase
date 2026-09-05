//! Wires `crates/jsvm` (a finished, decoupled QuickJS runtime) into the
//! server: this file is the [`cratebase_jsvm::HostApi`] implementation the
//! runtime calls `$app.*` / `$http.send` / `routerAdd` / `cronAdd` / `on*`
//! through, plus the glue that starts the runtime and turns its
//! registrations into real axum routes and cron jobs.
//!
//! # Two hosts, one executor split
//!
//! A JS handler bound to a record or collection write-path hook receives
//! `e.app`, and any `$app.save`/`$app.delete` it does through that handle
//! must join the same transaction as the operation that triggered the
//! hook (exactly like a native handler receiving [`TxApp`]). Everywhere
//! else — routes, crons, lifecycle hooks — there is no enclosing
//! transaction, so `$app` should talk to the plain [`App`]. [`JsvmHost`]
//! is generic over which one ([`HostExec`]) so both cases share one
//! `HostApi` implementation; [`wrap_host`] and [`wrap_tx_host`] build the
//! `Arc<dyn HostApi>` for each case.
//!
//! # Why hook registration needs a cell, not the `Runtime` itself
//!
//! [`Runtime::start`] evaluates every `*.pb.js` file — which is where
//! `on*`/`routerAdd`/`cronAdd` calls run and reach [`HostApi::register_hook`]
//! etc. — *before* it returns the [`Runtime`] handle those registrations
//! need to call back into later. [`App::jsvm_cell`] hands out the
//! `Arc<OnceLock<Runtime>>` that will hold it, so a hook bound while the
//! files are still being evaluated resolves the runtime lazily, at first
//! use, by which point `App::bootstrap` has stored it.
//!
//! # Routes
//!
//! `routerAdd` calls do not need the same trick: they are recorded on
//! [`App`] as plain data ([`JsRoute`]) and only turned into axum routes by
//! [`js_router`], which `crate::router` calls once every hook file has
//! already run.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter};
use axum::Router;
use cratebase_core::{AppError, Collection, Record, Settings};
use cratebase_db::context::{CollectionResolver, RequestContext};
use cratebase_db::engine::Executor;
use cratebase_db::{query, records};
use cratebase_jsvm::{
    CronHandlerId, HookHandlerId, HookKind, HostApi, HttpRequest, HttpResponse, JsBody, JsRequest,
    JsResponse, RecordTokenKind, Runtime, RuntimeConfig, TransactionFn, JS_RECORD_OPTIONS,
};
use cratebase_mailer::Message;
use serde_json::{Map, Value};

use crate::app::{App, JsRoute, TxApp};
use crate::extract::RequestInfo;
use crate::http_error::ApiError;

/// What a [`JsvmHost`] calls through: the plain database engine, or an
/// open transaction when the call originates inside a record/collection
/// write-path hook.
trait HostExec: Clone + Send + Sync + 'static {
    fn app(&self) -> &App;
    fn executor(&self) -> &dyn Executor;
}

impl HostExec for App {
    fn app(&self) -> &App {
        self
    }
    fn executor(&self) -> &dyn Executor {
        self.db()
    }
}

impl HostExec for TxApp {
    fn app(&self) -> &App {
        TxApp::app(self)
    }
    fn executor(&self) -> &dyn Executor {
        self
    }
}

/// `HostApi`, generic over which executor `$app.*` calls join.
struct JsvmHost<X: HostExec>(X);

/// Wrap the plain `App`: routes, crons, and every hook that does not
/// carry a `TxApp`.
pub fn wrap_host(app: App) -> Arc<dyn HostApi> {
    Arc::new(JsvmHost(app))
}

/// Wrap an open transaction: record/collection write-path hooks, so a
/// handler's own `$app.save`/`$app.delete` commits or rolls back with the
/// operation that triggered it.
pub fn wrap_tx_host(tx: TxApp) -> Arc<dyn HostApi> {
    Arc::new(JsvmHost(tx))
}

/// A process-wide string-keyed JSON store backing `$app.store()`, kept in
/// `App::store()` (a `TypeId`-keyed side table) under its own type so it
/// costs nothing when no `pb_hooks` file ever touches it.
#[derive(Default)]
struct JsStoreState(std::sync::RwLock<HashMap<String, Value>>);

/// Replace `{:name}` filter placeholders with literal values, exactly like
/// `cratebase_db::records`'s (private) helper of the same job: `$app.
/// findRecordsByFilter`'s `params` argument fills a hand-written filter
/// string rather than requiring string concatenation from JavaScript.
fn substitute_filter_params(filter: &str, params: &Map<String, Value>) -> String {
    if params.is_empty() || !filter.contains("{:") {
        return filter.to_string();
    }
    let mut out = filter.to_string();
    for (key, value) in params {
        let literal = match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            other => {
                let s = match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
            }
        };
        out = out.replace(&format!("{{:{key}}}"), &literal);
    }
    out
}

#[async_trait]
impl<X: HostExec> HostApi for JsvmHost<X> {
    fn settings(&self) -> Settings {
        (*self.0.app().settings()).clone()
    }

    async fn find_collection(&self, name_or_id: &str) -> Result<Arc<Collection>, AppError> {
        self.0
            .app()
            .db()
            .collections
            .get(name_or_id)
            .ok_or_else(|| AppError::not_found("Missing or invalid collection context."))
    }

    async fn find_record_by_id(&self, collection: &str, id: &str) -> Result<Record, AppError> {
        let col = self.find_collection(collection).await?;
        records::find_by_id_raw(self.0.executor(), &col, id)
            .await
            .map_err(AppError::from)
    }

    async fn find_records_by_filter(
        &self,
        collection: &str,
        filter: &str,
        sort: &str,
        limit: i64,
        offset: i64,
        params: Map<String, Value>,
    ) -> Result<Vec<Record>, AppError> {
        let col = self.find_collection(collection).await?;
        let app = self.0.app();
        let ex = self.0.executor();
        let ctx = RequestContext::superuser();
        let resolver =
            CollectionResolver::new(col.clone(), &app.db().collections, &ctx, ex.dialect());

        let expr = substitute_filter_params(filter, &params);
        let mut compiled_query = query::Query::new(&col);
        if !expr.trim().is_empty() {
            let compiled = cratebase_filter::parse_and_compile(&expr, &resolver, 0)
                .map_err(|e| AppError::bad_request(e.to_string()))?;
            compiled_query.push_filter(compiled);
        }
        let sort = if sort.trim().is_empty() {
            None
        } else {
            Some(sort)
        };
        compiled_query.set_order_by(query::order_by(&resolver, sort).map_err(AppError::from)?);

        let sql = compiled_query.select_sql();
        let lim = if limit <= 0 {
            records::MAX_PER_PAGE
        } else {
            limit
        };
        compiled_query.bind_page(lim, offset.max(0));
        let rows = ex
            .query(&sql, compiled_query.params())
            .await
            .map_err(AppError::from)?;
        Ok(rows
            .iter()
            .map(|r| records::row_to_record(&col, r))
            .collect())
    }

    async fn save_record(&self, mut record: Record) -> Result<Record, AppError> {
        let store = &self.0.app().db().collections;
        if record.is_new() {
            records::create(self.0.executor(), store, &mut record)
                .await
                .map_err(AppError::from)?;
        } else {
            records::update(self.0.executor(), store, &mut record)
                .await
                .map_err(AppError::from)?;
        }
        Ok(record)
    }

    async fn delete_record(&self, record: Record) -> Result<(), AppError> {
        let store = &self.0.app().db().collections;
        records::delete(self.0.executor(), store, &record)
            .await
            .map(|_| ())
            .map_err(AppError::from)
    }

    async fn save_collection(&self, collection: Collection) -> Result<Collection, AppError> {
        let app = self.0.app();
        let store = &app.db().collections;
        let engine = app.db().engine.as_ref();
        let saved = if store.get_by_id(&collection.id).is_some() {
            store.update(engine, &collection).await
        } else {
            store.insert(engine, &collection).await
        }
        .map_err(AppError::from)?;
        Ok((*saved).clone())
    }

    async fn delete_collection(&self, name_or_id: &str) -> Result<(), AppError> {
        let app = self.0.app();
        app.db()
            .collections
            .delete(app.db().engine.as_ref(), name_or_id)
            .await
            .map_err(AppError::from)
    }

    async fn run_in_transaction(&self, f: TransactionFn) -> Result<(), AppError> {
        self.0
            .app()
            .run_in_transaction(move |tx_app| f(wrap_tx_host(tx_app)))
            .await
    }

    async fn send_mail(
        &self,
        to: Vec<(String, String)>,
        subject: String,
        html: String,
    ) -> Result<(), AppError> {
        let mailer = self.0.app().mailer();
        let msg = Message {
            to,
            from: mailer.sender().clone(),
            subject,
            html,
            ..Default::default()
        };
        mailer
            .send(&msg)
            .await
            .map_err(|e| AppError::internal(e.to_string()))
    }

    async fn http_send(&self, req: HttpRequest) -> Result<HttpResponse, AppError> {
        static CLIENT: std::sync::LazyLock<reqwest::Client> =
            std::sync::LazyLock::new(reqwest::Client::new);
        let client = &*CLIENT;

        let method = req
            .method
            .parse::<reqwest::Method>()
            .map_err(|e| AppError::bad_request(e.to_string()))?;
        let mut builder = client.request(method, &req.url);
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        if req.timeout_secs > 0 {
            builder = builder.timeout(std::time::Duration::from_secs(req.timeout_secs));
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        let status = resp.status().as_u16();
        let mut headers = HashMap::new();
        for (name, value) in resp.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.to_string(), v.to_string());
            }
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
            .to_vec();
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    fn log(&self, level: i32, message: &str, data: Value) {
        match level {
            l if l >= 8 => tracing::error!(target: "jsvm", %data, "{message}"),
            l if l >= 4 => tracing::warn!(target: "jsvm", %data, "{message}"),
            l if l <= -4 => tracing::debug!(target: "jsvm", %data, "{message}"),
            _ => tracing::info!(target: "jsvm", %data, "{message}"),
        }
    }

    fn store_get(&self, key: &str) -> Option<Value> {
        let state = self
            .0
            .app()
            .store()
            .get_or_insert_with(JsStoreState::default);
        let value = state
            .0
            .read()
            .expect("jsvm store poisoned")
            .get(key)
            .cloned();
        value
    }

    fn store_set(&self, key: &str, value: Value) {
        let state = self
            .0
            .app()
            .store()
            .get_or_insert_with(JsStoreState::default);
        state
            .0
            .write()
            .expect("jsvm store poisoned")
            .insert(key.to_string(), value);
    }

    fn register_route(&self, method: &str, path: &str, handler: cratebase_jsvm::RouteHandlerId) {
        self.0.app().push_js_route(JsRoute {
            method: method.to_string(),
            pattern: path.to_string(),
            handler,
        });
    }

    fn register_cron(&self, id: &str, expr: &str, handler: CronHandlerId) {
        let cron_app = self.0.app().clone();
        let id_owned = id.to_string();
        let result = self.0.app().cron().add(id, expr, move || {
            let app = cron_app.clone();
            let handler = handler.clone();
            async move {
                let Some(rt) = app.jsvm() else { return };
                if let Err(e) = rt.call_cron(&handler).await {
                    tracing::warn!(error = %e, cron = %handler, "jsvm cron job failed");
                }
            }
        });
        if let Err(e) = result {
            tracing::warn!(error = %e, id = %id_owned, "invalid cron expression from pb_hooks");
        }
    }

    fn remove_cron(&self, id: &str) {
        self.0.app().cron().remove(id);
    }

    fn register_hook(
        &self,
        hook: HookKind,
        tags: Vec<String>,
        handler: HookHandlerId,
        priority: i32,
    ) {
        let app = self.0.app().clone();
        let cell = app.jsvm_cell();
        macro_rules! bind {
            ($field:ident) => {
                crate::hooks::bind_js_hook(&app.hooks().$field, cell, handler, tags, priority)
            };
        }
        match hook {
            HookKind::Bootstrap => bind!(on_bootstrap),
            HookKind::Serve => bind!(on_serve),
            HookKind::Terminate => bind!(on_terminate),

            HookKind::RecordEnrich => bind!(on_record_enrich),
            HookKind::RecordValidate => bind!(on_record_validate),

            HookKind::RecordCreate => bind!(on_record_create),
            HookKind::RecordCreateExecute => bind!(on_record_create_execute),
            HookKind::RecordAfterCreateSuccess => bind!(on_record_after_create_success),
            HookKind::RecordAfterCreateError => bind!(on_record_after_create_error),
            HookKind::RecordUpdate => bind!(on_record_update),
            HookKind::RecordUpdateExecute => bind!(on_record_update_execute),
            HookKind::RecordAfterUpdateSuccess => bind!(on_record_after_update_success),
            HookKind::RecordAfterUpdateError => bind!(on_record_after_update_error),
            HookKind::RecordDelete => bind!(on_record_delete),
            HookKind::RecordDeleteExecute => bind!(on_record_delete_execute),
            HookKind::RecordAfterDeleteSuccess => bind!(on_record_after_delete_success),
            HookKind::RecordAfterDeleteError => bind!(on_record_after_delete_error),

            HookKind::RecordCreateRequest => bind!(on_record_create_request),
            HookKind::RecordUpdateRequest => bind!(on_record_update_request),
            HookKind::RecordDeleteRequest => bind!(on_record_delete_request),
            HookKind::RecordListRequest => bind!(on_record_list_request),
            HookKind::RecordViewRequest => bind!(on_record_view_request),

            HookKind::CollectionValidate => bind!(on_collection_validate),
            HookKind::CollectionCreate => bind!(on_collection_create),
            HookKind::CollectionCreateExecute => bind!(on_collection_create_execute),
            HookKind::CollectionAfterCreateSuccess => bind!(on_collection_after_create_success),
            HookKind::CollectionAfterCreateError => bind!(on_collection_after_create_error),
            HookKind::CollectionUpdate => bind!(on_collection_update),
            HookKind::CollectionUpdateExecute => bind!(on_collection_update_execute),
            HookKind::CollectionAfterUpdateSuccess => bind!(on_collection_after_update_success),
            HookKind::CollectionAfterUpdateError => bind!(on_collection_after_update_error),
            HookKind::CollectionDelete => bind!(on_collection_delete),
            HookKind::CollectionDeleteExecute => bind!(on_collection_delete_execute),
            HookKind::CollectionAfterDeleteSuccess => bind!(on_collection_after_delete_success),
            HookKind::CollectionAfterDeleteError => bind!(on_collection_after_delete_error),
            HookKind::CollectionCreateRequest => bind!(on_collection_create_request),
            HookKind::CollectionUpdateRequest => bind!(on_collection_update_request),
            HookKind::CollectionDeleteRequest => bind!(on_collection_delete_request),
            HookKind::CollectionsListRequest => bind!(on_collections_list_request),
            HookKind::CollectionViewRequest => bind!(on_collection_view_request),
            HookKind::CollectionsImportRequest => bind!(on_collections_import_request),

            HookKind::RecordAuthRequest => bind!(on_record_auth_request),
            HookKind::RecordAuthWithPasswordRequest => bind!(on_record_auth_with_password_request),
            HookKind::RecordAuthWithOAuth2Request => bind!(on_record_auth_with_oauth2_request),
            HookKind::RecordAuthWithOTPRequest => bind!(on_record_auth_with_otp_request),
            HookKind::RecordAuthRefreshRequest => bind!(on_record_auth_refresh_request),
            HookKind::RecordRequestOTPRequest => bind!(on_record_request_otp_request),
            HookKind::RecordRequestPasswordResetRequest => {
                bind!(on_record_request_password_reset_request)
            }
            HookKind::RecordConfirmPasswordResetRequest => {
                bind!(on_record_confirm_password_reset_request)
            }
            HookKind::RecordRequestVerificationRequest => {
                bind!(on_record_request_verification_request)
            }
            HookKind::RecordConfirmVerificationRequest => {
                bind!(on_record_confirm_verification_request)
            }
            HookKind::RecordRequestEmailChangeRequest => {
                bind!(on_record_request_email_change_request)
            }
            HookKind::RecordConfirmEmailChangeRequest => {
                bind!(on_record_confirm_email_change_request)
            }

            HookKind::MailerSend => bind!(on_mailer_send),
            HookKind::MailerRecordVerificationSend => bind!(on_mailer_record_verification_send),
            HookKind::MailerRecordPasswordResetSend => bind!(on_mailer_record_password_reset_send),
            HookKind::MailerRecordEmailChangeSend => bind!(on_mailer_record_email_change_send),
            HookKind::MailerRecordOTPSend => bind!(on_mailer_record_otp_send),
            HookKind::MailerRecordAuthAlertSend => bind!(on_mailer_record_auth_alert_send),

            HookKind::RealtimeConnectRequest => bind!(on_realtime_connect_request),
            HookKind::RealtimeSubscribeRequest => bind!(on_realtime_subscribe_request),
            HookKind::RealtimeMessageSend => bind!(on_realtime_message_send),

            HookKind::FileDownloadRequest => bind!(on_file_download_request),
            HookKind::FileTokenRequest => bind!(on_file_token_request),

            HookKind::BackupCreate => bind!(on_backup_create),
            HookKind::BackupRestore => bind!(on_backup_restore),

            HookKind::SettingsListRequest => bind!(on_settings_list_request),
            HookKind::SettingsUpdateRequest => bind!(on_settings_update_request),
            HookKind::SettingsReload => bind!(on_settings_reload),

            HookKind::BatchRequest => bind!(on_batch_request),
        }
    }

    fn unregister_hook(&self, id: &str) {
        self.0.app().hooks().unbind_all_by_id(id);
    }

    async fn create_record_token(
        &self,
        record: &Record,
        kind: RecordTokenKind,
    ) -> Result<String, AppError> {
        let token_type = match kind {
            RecordTokenKind::Auth => cratebase_auth::TokenType::Auth,
            RecordTokenKind::Verification => cratebase_auth::TokenType::Verification,
            RecordTokenKind::PasswordReset => cratebase_auth::TokenType::PasswordReset,
            RecordTokenKind::EmailChange => cratebase_auth::TokenType::EmailChange,
            RecordTokenKind::File => cratebase_auth::TokenType::File,
        };
        self.0
            .app()
            .mint_token(&record.collection().name, record.id(), token_type, 0)
            .await
    }

    async fn validate_password(&self, record: &Record, password: &str) -> Result<bool, AppError> {
        let hash = record.get("password").and_then(Value::as_str).unwrap_or("");
        Ok(cratebase_auth::verify_password(password, hash))
    }

    /// Overridden rather than relying on the trait's `__plainPassword`
    /// marker convention: hashing here immediately means the value
    /// `record.setPassword` stores already satisfies `records::is_hash`,
    /// so the ordinary write path (`records::create`/`update`) leaves it
    /// alone instead of needing to recognise and unwrap a marker object.
    fn hash_password(&self, password: &str) -> Value {
        match cratebase_auth::hash_password(password) {
            Ok(hash) => Value::String(hash),
            Err(e) => {
                tracing::warn!(error = %e, "password hashing failed for record.setPassword");
                Value::String(String::new())
            }
        }
    }
}

/// `true` when `dir` exists and contains at least one `*.pb.js` file.
fn has_hook_files(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|t| t.is_file())
            && entry.file_name().to_string_lossy().ends_with(".pb.js")
    })
}

/// Starts the JS runtime when `pb_hooks/` exists and has at least one
/// `*.pb.js` file, and stores it on `app` so hooks bound during startup
/// (see [`bind_js_hook`](crate::hooks::bind_js_hook)) can resolve it and
/// [`js_router`] can mount its `routerAdd` routes. A complete no-op —
/// nothing is spawned, nothing is allocated beyond a directory read —
/// when `pb_hooks/` is absent or empty, matching PocketBase's own
/// behaviour.
pub async fn maybe_start(app: &App) -> Result<(), AppError> {
    let hooks_dir = std::path::PathBuf::from(&app.config().hooks_dir);
    if !has_hook_files(&hooks_dir) {
        return Ok(());
    }

    let mut cfg = RuntimeConfig::new(hooks_dir.clone(), app.config().migrations_dir.clone());
    cfg.hooks_watch = app.config().dev;
    cfg.types_file = Some(hooks_dir.join("types.d.ts"));

    let runtime = Runtime::start(wrap_host(app.clone()), cfg)
        .await
        .map_err(|e| AppError::internal(format!("failed to start the JS runtime: {e}")))?;
    // Set once, by `App::bootstrap`; a second call is refused there.
    let _ = app.jsvm_cell().set(runtime);
    Ok(())
}

/// PocketBase path segments (`{name}`, `{path...}`) to axum 0.8 ones
/// (`{name}`, `{*path}`); axum already uses `{name}` for a named
/// parameter, so only the trailing-wildcard form needs rewriting.
fn convert_pattern(pattern: &str) -> String {
    pattern
        .split('/')
        .map(
            |seg| match seg.strip_prefix('{').and_then(|s| s.strip_suffix("...}")) {
                Some(inner) => format!("{{*{inner}}}"),
                None => seg.to_string(),
            },
        )
        .collect::<Vec<_>>()
        .join("/")
}

/// A filter matching every method, for `routerAdd`'s "any method" form
/// (an empty or unrecognised method string).
fn any_method() -> MethodFilter {
    MethodFilter::GET
        .or(MethodFilter::HEAD)
        .or(MethodFilter::POST)
        .or(MethodFilter::PUT)
        .or(MethodFilter::PATCH)
        .or(MethodFilter::DELETE)
        .or(MethodFilter::OPTIONS)
        .or(MethodFilter::TRACE)
}

fn method_filter(method: &str) -> MethodFilter {
    if method.trim().is_empty() {
        return any_method();
    }
    Method::try_from(method.trim().to_ascii_uppercase().as_str())
        .ok()
        .and_then(|m| MethodFilter::try_from(m).ok())
        .unwrap_or_else(any_method)
}

/// Every `routerAdd` registration, mounted at the exact path it was
/// registered with (root-level, same as PocketBase — these are not
/// nested under `/api`). Empty, and therefore free, when no `pb_hooks`
/// file ever called `routerAdd`.
pub fn js_router(app: &App) -> Router<App> {
    let mut by_pattern: HashMap<String, MethodRouter<App>> = HashMap::new();
    for route in app.js_routes() {
        let pattern = convert_pattern(&route.pattern);
        let filter = method_filter(&route.method);
        let handler = route.handler;
        let method_router = by_pattern.remove(&pattern).unwrap_or_default().on(
            filter,
            move |State(app): State<App>,
                  Path(path_params): Path<HashMap<String, String>>,
                  ConnectInfo(peer): ConnectInfo<SocketAddr>,
                  headers: HeaderMap,
                  info: RequestInfo,
                  body: Bytes| {
                let handler = handler.clone();
                async move {
                    respond_js_route(app, handler, path_params, Some(peer), headers, info, body)
                        .await
                }
            },
        );
        by_pattern.insert(pattern, method_router);
    }

    let mut router = Router::new();
    for (pattern, method_router) in by_pattern {
        router = router.route(&pattern, method_router);
    }
    router
}

#[allow(clippy::too_many_arguments)]
async fn respond_js_route(
    app: App,
    handler: cratebase_jsvm::RouteHandlerId,
    path_params: HashMap<String, String>,
    peer: Option<SocketAddr>,
    headers: HeaderMap,
    info: RequestInfo,
    body: Bytes,
) -> Result<Response, ApiError> {
    let Some(runtime) = app.jsvm() else {
        // The runtime stopped (shutdown mid-request); answer like any
        // other route that no longer exists rather than panicking.
        return Err(ApiError::not_found(""));
    };

    let body_value: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
    };

    let req = JsRequest {
        method: info.method.clone(),
        path: info.path.clone(),
        query: info.query.clone(),
        headers: info.headers.clone(),
        body: body_value,
        auth: info
            .auth
            .as_ref()
            .map(|a| a.record.to_json(JS_RECORD_OPTIONS)),
        path_params: Some(
            path_params
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
        remote_ip: crate::middleware::client_ip::client_ip(
            &headers,
            peer,
            &app.settings().trusted_proxy,
        ),
    };

    let resp = runtime.call_route(&handler, req).await.map_err(ApiError)?;
    Ok(js_response_into_axum(resp))
}

fn js_response_into_axum(resp: JsResponse) -> Response {
    let status = StatusCode::from_u16(if resp.status == 0 { 200 } else { resp.status })
        .unwrap_or(StatusCode::OK);
    let mut builder = axum::http::Response::builder().status(status);
    let mut has_content_type = false;
    for (name, value) in &resp.headers {
        if name.eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) {
            builder = builder.header(name, value);
        }
    }

    let body = match resp.body {
        JsBody::Json(v) => {
            if !has_content_type {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
            }
            Body::from(serde_json::to_vec(&v).unwrap_or_default())
        }
        JsBody::Text(s) => {
            if !has_content_type {
                builder = builder.header(header::CONTENT_TYPE, "text/plain; charset=utf-8");
            }
            Body::from(s)
        }
        JsBody::Html(s) => {
            if !has_content_type {
                builder = builder.header(header::CONTENT_TYPE, "text/html; charset=utf-8");
            }
            Body::from(s)
        }
        JsBody::Bytes(b) => Body::from(b),
        JsBody::Redirect(url) => {
            builder = builder.header(header::LOCATION, url);
            Body::empty()
        }
        JsBody::NoContent => Body::empty(),
    };

    builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
