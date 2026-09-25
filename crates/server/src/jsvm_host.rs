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
use cratebase_db::engine::{Executor, Sql};
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
    /// `true` for a [`TxApp`]: `executor()` is (or may be) an already-open
    /// transaction, so a collection save/delete must join it through
    /// `CollectionStore::insert_with`/`update_with`/`delete_with` rather
    /// than opening its own nested one — see those methods' doc comments.
    fn is_transactional(&self) -> bool;
}

impl HostExec for App {
    fn app(&self) -> &App {
        self
    }
    fn executor(&self) -> &dyn Executor {
        self.db()
    }
    fn is_transactional(&self) -> bool {
        false
    }
}

impl HostExec for TxApp {
    fn app(&self) -> &App {
        TxApp::app(self)
    }
    fn executor(&self) -> &dyn Executor {
        self
    }
    fn is_transactional(&self) -> bool {
        true
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

/// PocketBase's own `$http.send` defaults its `timeout` option to 120
/// (seconds) when the caller doesn't set one; match that rather than
/// inventing our own number, since a script ported from PocketBase
/// should time out the same way it did there.
const DEFAULT_HTTP_SEND_TIMEOUT_SECS: u64 = 120;

/// The `reqwest` timeout to apply for a given `HttpRequest.timeout_secs`
/// (`0` meaning "unspecified"). Always bounded: an unbounded default
/// would let one hung outbound call block a JS worker indefinitely.
fn http_send_timeout(requested_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(if requested_secs > 0 {
        requested_secs
    } else {
        DEFAULT_HTTP_SEND_TIMEOUT_SECS
    })
}

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

/// Rewrite `{:name}` placeholders in a `$app.rawQuery` statement into the
/// engine's positional `$1..$n` placeholders (see
/// `cratebase_db::engine`'s module doc), binding each occurrence to
/// `params[name]` as a real driver-level [`Sql`] parameter. Unlike
/// [`substitute_filter_params`] above — which literal-substitutes into
/// filter-language text the filter compiler re-parses and binds for
/// real — this never inlines a value into the SQL string, so arbitrary
/// hook-supplied `sql` carries no more injection risk than a
/// hand-written parameterized query.
fn bind_raw_query_params(
    sql: &str,
    params: &Map<String, Value>,
) -> Result<(String, Vec<Sql>), AppError> {
    let mut out = String::with_capacity(sql.len());
    let mut bound: Vec<Sql> = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '{' && sql[i + 1..].starts_with(':') {
            let rest = &sql[i + 2..];
            let name_len = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            if name_len > 0 && rest[name_len..].starts_with('}') {
                let name = &rest[..name_len];
                let value = params.get(name).ok_or_else(|| {
                    AppError::bad_request(format!("rawQuery: missing parameter :{name}"))
                })?;
                bound.push(raw_query_sql_param(name, value)?);
                out.push('$');
                out.push_str(&bound.len().to_string());
                for _ in 0..(name_len + 2) {
                    chars.next();
                }
                continue;
            }
        }
        out.push(c);
    }
    Ok((out, bound))
}

/// A `$app.rawQuery` parameter value, restricted to what a single bound
/// column can hold — objects/arrays have no unambiguous SQL binding, so
/// they're rejected rather than silently stringified.
fn raw_query_sql_param(name: &str, value: &Value) -> Result<Sql, AppError> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Int(*b as i64),
        Value::Number(n) => n
            .as_i64()
            .map(Sql::Int)
            .or_else(|| n.as_f64().map(Sql::Real))
            .ok_or_else(|| {
                AppError::bad_request(format!(
                    "rawQuery: parameter :{name} is not a finite number"
                ))
            })?,
        Value::String(s) => Sql::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => {
            return Err(AppError::bad_request(format!(
                "rawQuery: parameter :{name} must be a string, number, boolean, or null"
            )))
        }
    })
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

    async fn raw_query(
        &self,
        sql: &str,
        params: Map<String, Value>,
    ) -> Result<Vec<Map<String, Value>>, AppError> {
        if !crate::routes::sql_console::is_read_statement(sql) {
            return Err(AppError::bad_request(
                "rawQuery: sql must start with SELECT or WITH; use $app.save/$app.delete for writes",
            ));
        }
        let (bound_sql, bound_params) = bind_raw_query_params(sql, &params)?;
        let rows = self
            .0
            .executor()
            .query(&bound_sql, &bound_params)
            .await
            .map_err(AppError::from)?;
        Ok(rows
            .into_iter()
            .map(crate::routes::sql_console::row_to_map)
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
        let exists = store.get_by_id(&collection.id).is_some();
        let saved = if self.0.is_transactional() {
            // Join the caller's already-open transaction instead of
            // opening a nested one — see `HostExec::is_transactional`.
            let ex = self.0.executor();
            if exists {
                store.update_with(ex, &collection).await
            } else {
                store.insert_with(ex, &collection).await
            }
        } else {
            let engine = app.db().engine.as_ref();
            if exists {
                store.update(engine, &collection).await
            } else {
                store.insert(engine, &collection).await
            }
        }
        .map_err(AppError::from)?;
        Ok((*saved).clone())
    }

    async fn delete_collection(&self, name_or_id: &str) -> Result<(), AppError> {
        let app = self.0.app();
        if self.0.is_transactional() {
            app.db()
                .collections
                .delete_with(self.0.executor(), name_or_id)
                .await
                .map_err(AppError::from)
        } else {
            app.db()
                .collections
                .delete(app.db().engine.as_ref(), name_or_id)
                .await
                .map_err(AppError::from)
        }
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

    async fn mails_send(&self, input: Map<String, Value>) -> Result<Value, AppError> {
        let get_str = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
        let recipients = |key: &str| -> Result<Vec<crate::mails::Recipient>, AppError> {
            crate::mails::parse_recipients(input.get(key).unwrap_or(&Value::Null))
                .map_err(AppError::bad_request)
        };
        let from = match input.get("from") {
            Some(v) if !v.is_null() => crate::mails::parse_recipients(v)
                .map_err(AppError::bad_request)?
                .into_iter()
                .next(),
            _ => None,
        };
        let send_input = crate::mails::SendInput {
            to: recipients("to")?,
            cc: recipients("cc")?,
            bcc: recipients("bcc")?,
            template: get_str("template"),
            locale: get_str("locale"),
            data: input.get("data").cloned().unwrap_or(Value::Null),
            subject: get_str("subject"),
            html: get_str("html"),
            text: get_str("text"),
            from,
            reply_to: get_str("replyTo"),
        };
        let outcome = crate::mails::send(self.0.app(), send_input).await?;
        serde_json::to_value(outcome).map_err(|e| AppError::internal(e.to_string()))
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
        // Always bounded: a hung outbound call must not tie up a JS
        // worker forever (see `http_send_timeout`'s doc for the default).
        builder = builder.timeout(http_send_timeout(req.timeout_secs));
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

    fn clear_routes(&self) {
        self.0.app().clear_js_routes();
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

/// `true` when `dir` exists and contains at least one `*.js` migration
/// file (any `.js` file — migrations do not use the `.pb.js` naming
/// hooks do; see `write_migration_stub` in `main.rs`).
fn has_migration_files(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|t| t.is_file())
            && entry.file_name().to_string_lossy().ends_with(".js")
    })
}

/// Starts the JS runtime when `pb_hooks/` has at least one `*.pb.js` file,
/// `pb_migrations/` has at least one `*.js` migration, or the server is
/// running `--dev`, and stores it on `app` so hooks bound during startup
/// (see [`bind_js_hook`](crate::hooks::bind_js_hook)) can resolve it,
/// [`js_router`] can mount its `routerAdd` routes, and `crate::js_migrations`
/// can apply/revert migrations. The runtime must start for migrations
/// even when there is no hook file at all — `pb_migrations` alone is a
/// perfectly normal setup — so this checks both directories.
///
/// `--dev` always starts it, even with both directories empty: hot
/// reload (the watcher `cfg.hooks_watch` arms below) only ever reloads a
/// *running* pool, so the very first `pb_hooks/*.pb.js` a developer adds
/// needs the runtime already up to be picked up without a restart.
/// Starting an otherwise-idle pool is cheap (a handful of worker threads
/// that evaluate zero files and then sit on the job queue), which is what
/// makes this an acceptable default for `--dev` specifically — it stays
/// opt-in for a production boot, where every idle thread is unwanted
/// cost, via the exact same "must contain something" check as before.
pub async fn maybe_start(app: &App) -> Result<(), AppError> {
    let hooks_dir = std::path::PathBuf::from(&app.config().hooks_dir);
    let migrations_dir = std::path::PathBuf::from(&app.config().migrations_dir);
    let dev = app.config().dev;
    if !dev && !has_hook_files(&hooks_dir) && !has_migration_files(&migrations_dir) {
        return Ok(());
    }

    let mut cfg = RuntimeConfig::new(hooks_dir.clone(), migrations_dir);
    cfg.hooks_watch = dev;
    // PocketBase's own convention (`crates/jsvm/src/types.rs`'s module
    // doc and `types.d.ts` itself): hook and migration files reference
    // `../pb_data/types.d.ts`, and `pb_data` is a sibling of both
    // `pb_hooks` and `pb_migrations` (see `config::sibling`), so the file
    // has to live under the data dir, not under `hooks_dir` as before.
    cfg.types_file = Some(std::path::PathBuf::from(&app.config().data_dir).join("types.d.ts"));

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

/// The dynamic dispatcher `crate::router` mounts as the top-level
/// fallback, so a `routerAdd`/reload cycle takes effect on the very next
/// request with no restart and no rebuilt/cached router to invalidate.
///
/// Earlier, `js_router`'s output was merged straight into the assembled
/// `Router` once, at boot — exactly like every built-in route — which
/// baked the route *table*, not just the routes, into the server for the
/// rest of the process's life: a `pb_hooks` file added, changed or
/// deleted after that could reload the JS *runtime* (the watcher already
/// did that), but the axum router still dispatched by the stale table,
/// so the new/changed/removed route never took effect until a restart.
/// Building [`js_router`] fresh from `app.js_routes()` — itself just a
/// `Mutex<Vec<JsRoute>>` snapshot, kept in sync by [`App::clear_js_routes`]
/// and `register_route` — on every fallback invocation instead means
/// there is no separate table to go stale: whatever the runtime most
/// recently reloaded *is* what the next request sees.
///
/// This only runs at all once nothing built in — `/api`, the dashboard,
/// `/metrics` — has already matched the request by path (that's what
/// makes it the router's `fallback`), so those routes keep winning over a
/// same-path `routerAdd` exactly as before; a JS route is a pure
/// addition, never able to shadow one.
pub fn js_fallback_router(app: &App) -> Router {
    Router::new()
        .fallback(dispatch_js_route)
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::middleware::rate_limit::rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::middleware::csrf::csrf,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            crate::middleware::request_log::log_requests,
        ))
        .with_state(app.clone())
}

/// Build the current `routerAdd` table into a router, self-contained with
/// PocketBase's own "no matching route or method is a 404, not a 405"
/// answer (matching `crate::router`'s handling of the built-in routes),
/// and dispatch straight to it. Whatever it returns — a real handler's
/// response, or its own not-found fallback — is exactly the response
/// [`js_fallback_router`] should send, so there is no result left to
/// interpret afterwards.
async fn dispatch_js_route(
    State(app): State<App>,
    req: axum::extract::Request,
) -> Result<Response, std::convert::Infallible> {
    let router = js_router(&app)
        .fallback(crate::http_error::not_found_fallback)
        .method_not_allowed_fallback(crate::http_error::not_found_fallback)
        .with_state(app);
    tower::ServiceExt::oneshot(router, req).await
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
        raw_body: String::from_utf8_lossy(&body).into_owned(),
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

#[cfg(test)]
mod raw_query_tests {
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::{bind_raw_query_params, Map, Sql};
    use crate::app::App;
    use crate::config::Config;

    async fn test_app_with_hook(hook_js: &str) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        // `Config::memory`'s `hooks_dir`/`migrations_dir` are *siblings*
        // of the data dir (see `config::sibling`), so passing `dir.path()`
        // itself as the data dir would put `pb_hooks` at `dir.path()`'s
        // *parent* — `/tmp` — shared by every test in the binary that
        // does the same. A `pb_data` subdirectory keeps every derived
        // path under this test's own unique tempdir.
        let cfg = Config::memory(dir.path().join("pb_data"));
        std::fs::create_dir_all(&cfg.hooks_dir).expect("create hooks dir");
        std::fs::write(
            std::path::Path::new(&cfg.hooks_dir).join("main.pb.js"),
            hook_js,
        )
        .expect("write hook file");
        let app = App::new(cfg);
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    /// `routerAdd`'s handlers require a `ConnectInfo<SocketAddr>` request
    /// extension (see `js_router`), which only a real listener normally
    /// supplies; a `Router::oneshot` test has to set it explicitly.
    fn get(uri: &str) -> Request<Body> {
        let mut req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
        req
    }

    /// End-to-end proof of the issue's actual ask: a `routerAdd` handler
    /// reads through `$app.rawQuery` and gets back a plain object with no
    /// `Record` wrapper, bound through a named `{:email}` placeholder
    /// rather than string concatenation.
    #[tokio::test]
    async fn raw_query_returns_plain_rows_bound_by_name() {
        let hook = r#"
            routerAdd("GET", "/raw-query-test", (e) => {
                const rows = $app.rawQuery(
                    "SELECT id, email FROM _superusers WHERE email = {:email}",
                    { email: "rawquery-test@example.com" }
                );
                e.json(200, { rows });
            });
        "#;
        let (app, _dir) = test_app_with_hook(hook).await;
        app.create_superuser("rawquery-test@example.com", "password12345")
            .await
            .expect("create superuser");

        let router = crate::router(app);
        let response = router.oneshot(get("/raw-query-test")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let rows = json["rows"].as_array().expect("rows array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["email"], "rawquery-test@example.com");
        assert!(rows[0]["id"].is_string());
        // No Record wrapper: only the selected columns come back, none of
        // Record's `collectionId`/`collectionName`/etc. bookkeeping.
        assert_eq!(rows[0].as_object().unwrap().len(), 2);
    }

    /// The read-only gate: a write statement is rejected server-side, not
    /// merely left to the caller's discipline.
    #[tokio::test]
    async fn raw_query_rejects_non_select_statements() {
        let hook = r#"
            routerAdd("GET", "/raw-query-write", (e) => {
                try {
                    $app.rawQuery("DELETE FROM _superusers");
                    e.json(200, { ok: true });
                } catch (err) {
                    e.json(400, { error: String(err.message || err) });
                }
            });
        "#;
        let (app, _dir) = test_app_with_hook(hook).await;
        let router = crate::router(app);
        let response = router.oneshot(get("/raw-query-write")).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"].as_str().unwrap().contains("SELECT"));
    }

    #[test]
    fn bind_raw_query_params_binds_named_placeholders_positionally() {
        let params = serde_json::json!({ "a": 1, "b": "x" })
            .as_object()
            .unwrap()
            .clone();
        let (sql, bound) =
            bind_raw_query_params("SELECT * FROM t WHERE x = {:a} AND y = {:b}", &params).unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE x = $1 AND y = $2");
        assert_eq!(bound, vec![Sql::Int(1), Sql::Text("x".into())]);
    }

    #[test]
    fn bind_raw_query_params_errors_on_missing_param() {
        let params = Map::new();
        let err =
            bind_raw_query_params("SELECT * FROM t WHERE x = {:missing}", &params).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}

/// `HostApi::mails_send`'s glue — called directly (not through the JS
/// runtime) since the `hostCall("mailsSend", ...)` dispatch it sits
/// behind is exercised the same mechanical way as every other hostCall
/// in `cratebase_jsvm::bridge`, and the payload shape it produces
/// (`{ to, template, data, ... }` in, `{ id, status, error }` out) is the
/// same one `crate::routes::mails` builds from JSON — see
/// `crate::mails`'s own tests for the pipeline itself.
#[cfg(test)]
mod mails_send_tests {
    use super::{wrap_host, Map, Value};
    use crate::app::App;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    #[tokio::test]
    async fn sends_a_template_mail_through_the_real_pipeline() {
        let (app, _dir) = test_app().await;
        let host = wrap_host(app.clone());
        let mut input = Map::new();
        input.insert("to".into(), Value::String("someone@example.com".into()));
        input.insert("template".into(), Value::String("welcome".into()));
        input.insert(
            "data".into(),
            serde_json::json!({ "user": { "name": "Bob" } }),
        );
        let result = host.mails_send(input).await.expect("mails_send");
        assert_eq!(result["status"], "sent");
        assert!(result["id"].as_str().is_some_and(|s| !s.is_empty()));

        let mailbox = app.mailer().dev_mailbox().expect("dev mailbox in tests");
        assert_eq!(mailbox.len(), 1);
        assert!(mailbox.list()[0].subject.contains("Welcome"));
    }

    #[tokio::test]
    async fn rejects_an_invalid_recipient() {
        let (app, _dir) = test_app().await;
        let host = wrap_host(app);
        let mut input = Map::new();
        input.insert("to".into(), Value::String("not-an-email".into()));
        input.insert("subject".into(), Value::String("S".into()));
        input.insert("html".into(), Value::String("<p>hi</p>".into()));
        let err = host.mails_send(input).await.unwrap_err();
        assert!(err.to_string().contains("valid email"));
    }
}

#[cfg(test)]
mod http_send_tests {
    use super::http_send_timeout;

    #[test]
    fn defaults_to_pocketbases_120s_timeout_when_unset() {
        assert_eq!(
            http_send_timeout(0),
            std::time::Duration::from_secs(120),
            "an unbounded default would let a hung call block a JS worker forever"
        );
    }

    #[test]
    fn keeps_an_explicit_timeout() {
        assert_eq!(http_send_timeout(5), std::time::Duration::from_secs(5));
    }
}

/// The actual ask behind this module's `js_fallback_router`/`maybe_start`
/// changes: a `--dev` app picks up a brand-new `routerAdd`/`cronAdd`
/// registration with no restart, because (1) the runtime is already
/// running even with an empty `pb_hooks/` and (2) the router dispatches
/// JS routes by looking up the *current* table on every request instead
/// of one baked in at boot.
#[cfg(test)]
mod hot_reload_tests {
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::App;
    use crate::config::Config;

    fn get(uri: &str) -> Request<Body> {
        let mut req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
        req
    }

    async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = body_bytes(resp).await;
        if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        }
    }

    fn json_request(
        method: &str,
        uri: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", token)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
        req
    }

    fn authed_get(uri: &str, token: &str) -> Request<Body> {
        let mut req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", token)
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
        req
    }

    /// Retries `check` every 50ms until it returns `true` or `timeout`
    /// elapses, for tests driven by `spawn_watcher`'s background polling
    /// thread rather than a synchronous `Runtime::reload()` call.
    async fn poll_until<F, Fut>(timeout: std::time::Duration, mut check: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if check().await {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// A superuser token for a freshly bootstrapped `app`, for tests that
    /// need to create collections or write records through the real HTTP
    /// router rather than reaching into `App` internals directly.
    async fn superuser_token(app: &App) -> String {
        let id = app
            .create_superuser("admin@example.com", "hunter2hunter2")
            .await
            .expect("superuser");
        app.mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("token")
    }

    /// A `--dev` app booted against an *empty* `pb_hooks/` directory. Hot
    /// reload only ever reloads an already-running pool (`Runtime::reload`
    /// re-evaluates files on the existing workers), so this is only
    /// possible at all when `maybe_start` starts the runtime for `--dev`
    /// unconditionally — see that function's doc.
    async fn dev_app_with_empty_hooks() -> (App, tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut cfg = Config::memory(dir.path().join("pb_data"));
        cfg.dev = true;
        std::fs::create_dir_all(&cfg.hooks_dir).expect("create hooks dir");
        let hooks_dir = std::path::PathBuf::from(&cfg.hooks_dir);
        let app = App::new(cfg);
        app.bootstrap().await.expect("bootstrap");
        (app, dir, hooks_dir)
    }

    #[tokio::test]
    async fn dev_starts_the_runtime_even_with_no_hook_files() {
        let (app, _dir, _hooks) = dev_app_with_empty_hooks().await;
        assert!(
            app.jsvm().is_some(),
            "--dev must start the runtime even with an empty pb_hooks/, so the first \
             hook file added later can be picked up without a restart"
        );
    }

    #[tokio::test]
    async fn routeradd_added_changed_and_removed_takes_effect_after_reload_with_no_restart() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        // Built once, like the real server does — proving the *router*,
        // not just the runtime, needs no restart.
        let router = crate::router(app.clone());
        let hook_file = hooks_dir.join("main.pb.js");

        let resp = router.clone().oneshot(get("/hello")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "no routerAdd yet");

        std::fs::write(
            &hook_file,
            r#"routerAdd("GET", "/hello/{name}", (e) => e.string(200, "v1:" + e.request.pathValue("name")));"#,
        )
        .unwrap();
        app.jsvm()
            .unwrap()
            .reload()
            .await
            .expect("reload after add");

        let resp = router.clone().oneshot(get("/hello/world")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, b"v1:world");

        std::fs::write(
            &hook_file,
            r#"routerAdd("GET", "/hello/{name}", (e) => e.string(200, "v2:" + e.request.pathValue("name")));"#,
        )
        .unwrap();
        app.jsvm()
            .unwrap()
            .reload()
            .await
            .expect("reload after change");
        let resp = router.clone().oneshot(get("/hello/world")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, b"v2:world");

        std::fs::remove_file(&hook_file).unwrap();
        app.jsvm()
            .unwrap()
            .reload()
            .await
            .expect("reload after remove");
        let resp = router.oneshot(get("/hello/world")).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "a routerAdd removed by the reloaded files must stop matching"
        );
    }

    #[tokio::test]
    async fn built_in_and_dashboard_routes_still_win_over_a_same_path_js_route() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        let router = crate::router(app.clone());
        std::fs::write(
            hooks_dir.join("main.pb.js"),
            r#"routerAdd("GET", "/api/health", (e) => e.string(200, "js-hijack"));"#,
        )
        .unwrap();
        app.jsvm().unwrap().reload().await.expect("reload");

        // The built-in `/api/health` must still answer, never the JS
        // route registered at the same path.
        let resp = router.oneshot(get("/api/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_bytes(resp).await;
        assert_ne!(&body[..], b"js-hijack");
    }

    #[tokio::test]
    async fn reload_does_not_duplicate_cron_jobs() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        std::fs::write(
            hooks_dir.join("main.pb.js"),
            r#"cronAdd("seedHotReloadJob", "0 0 * * *", () => {});"#,
        )
        .unwrap();
        let rt = app.jsvm().unwrap();
        rt.reload().await.expect("reload 1");
        rt.reload().await.expect("reload 2");
        rt.reload().await.expect("reload 3");

        let matches = app
            .cron()
            .list()
            .into_iter()
            .filter(|j| j.id == "seedHotReloadJob")
            .count();
        assert_eq!(
            matches, 1,
            "reload must not duplicate an unchanged cron job"
        );
    }

    #[tokio::test]
    async fn reload_drops_a_cron_job_the_reloaded_files_no_longer_register() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        let hook_file = hooks_dir.join("main.pb.js");
        std::fs::write(
            &hook_file,
            r#"cronAdd("goneAfterReload", "0 0 * * *", () => {});"#,
        )
        .unwrap();
        let rt = app.jsvm().unwrap();
        rt.reload().await.expect("reload with the job present");
        assert!(app.cron().has("goneAfterReload"));

        std::fs::write(&hook_file, "// no more cronAdd here\n").unwrap();
        rt.reload().await.expect("reload without the job");
        assert!(
            !app.cron().has("goneAfterReload"),
            "a cron job a reloaded file no longer registers must be removed"
        );
    }

    /// Dogfooding bug report: editing a `pb_hooks/*.pb.js` file while
    /// `--dev` runs reportedly left record-hook *enforcement* stuck on
    /// the old file's behavior until a restart, unlike `routerAdd`/
    /// `cronAdd` above (already covered, and already working). Reproduce
    /// it the same way as those: bind `onRecordCreate` to reject a
    /// create missing a field, confirm the rejection, rewrite the file
    /// with a different message, `Runtime::reload()`, and confirm the
    /// *new* message is what a create now gets — proving the record hook
    /// itself, not just the router/cron tables, is rebuilt by reload.
    #[tokio::test]
    async fn record_hook_enforcement_and_message_change_after_edit_take_effect_on_reload() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        let router = crate::router(app.clone());
        let token = superuser_token(&app).await;

        let create_collection = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/collections",
                &token,
                serde_json::json!({
                    "name": "widgets",
                    "type": "base",
                    "listRule": "",
                    "viewRule": "",
                    "createRule": "",
                    "updateRule": "",
                    "deleteRule": "",
                    "fields": [{"name": "name", "type": "text"}],
                }),
            ))
            .await
            .unwrap();
        assert_eq!(create_collection.status(), StatusCode::OK);

        let hook_file = hooks_dir.join("main.pb.js");
        std::fs::write(
            &hook_file,
            r#"onRecordCreate((e) => {
                if (!e.record.get("name")) { throw new BadRequestError("name is required v1"); }
                e.next();
            }, "widgets");"#,
        )
        .unwrap();
        app.jsvm().unwrap().reload().await.expect("initial reload");

        let rejected = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/collections/widgets/records",
                &token,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let body = body_json(rejected).await;
        assert_eq!(body["message"], "name is required v1", "{body}");

        std::fs::write(
            &hook_file,
            r#"onRecordCreate((e) => {
                if (!e.record.get("name")) { throw new BadRequestError("name is required v2"); }
                e.next();
            }, "widgets");"#,
        )
        .unwrap();
        app.jsvm()
            .unwrap()
            .reload()
            .await
            .expect("reload after editing the hook file");

        let rejected_again = router
            .oneshot(json_request(
                "POST",
                "/api/collections/widgets/records",
                &token,
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected_again.status(), StatusCode::BAD_REQUEST);
        let body = body_json(rejected_again).await;
        assert_eq!(
            body["message"], "name is required v2",
            "a rewritten hook file's message must take effect after reload, not the stale one: {body}"
        );
    }

    /// Dogfooding bug report: `onRecordAfterUpdateSuccess` reportedly
    /// never fired on `PATCH` at all — independent of hot reload, so
    /// this is checked with no reload in between first — and, per the
    /// bug report, possibly the same root cause as the reload issue
    /// above, so it is checked again *after* a reload too. The hook
    /// writes a `markers` row (rather than just observing) so a false
    /// pass can't come from an event that "fires" with nothing to prove
    /// it actually ran and reached `$app.save`.
    #[tokio::test]
    async fn on_record_after_update_success_fires_on_patch_before_and_after_reload() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        let router = crate::router(app.clone());
        let token = superuser_token(&app).await;

        for (name, fields) in [
            (
                "widgets",
                serde_json::json!([{"name": "name", "type": "text"}]),
            ),
            (
                "markers",
                serde_json::json!([{"name": "note", "type": "text"}]),
            ),
        ] {
            let resp = router
                .clone()
                .oneshot(json_request(
                    "POST",
                    "/api/collections",
                    &token,
                    serde_json::json!({
                        "name": name,
                        "type": "base",
                        "listRule": "",
                        "viewRule": "",
                        "createRule": "",
                        "updateRule": "",
                        "deleteRule": "",
                        "fields": fields,
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "creating {name}");
        }

        let hook_file = hooks_dir.join("main.pb.js");
        std::fs::write(
            &hook_file,
            r#"onRecordAfterUpdateSuccess((e) => {
                const markers = $app.findCollectionByNameOrId("markers");
                const marker = new Record(markers, { note: "updated-v1:" + e.record.id });
                e.app.save(marker);
                e.next();
            }, "widgets");"#,
        )
        .unwrap();
        app.jsvm().unwrap().reload().await.expect("initial reload");

        let created = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/collections/widgets/records",
                &token,
                serde_json::json!({"name": "before"}),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);
        let created = body_json(created).await;
        let id = created["id"].as_str().unwrap().to_string();

        let updated = router
            .clone()
            .oneshot(json_request(
                "PATCH",
                &format!("/api/collections/widgets/records/{id}"),
                &token,
                serde_json::json!({"name": "after"}),
            ))
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);

        let markers_before_reload = router
            .clone()
            .oneshot(authed_get(
                &format!("/api/collections/markers/records?filter=note='updated-v1:{id}'"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(markers_before_reload.status(), StatusCode::OK);
        let markers_before_reload = body_json(markers_before_reload).await;
        assert_eq!(
            markers_before_reload["items"].as_array().unwrap().len(),
            1,
            "onRecordAfterUpdateSuccess must fire on PATCH and its $app.save must land: {markers_before_reload}"
        );

        // Same hook, rewritten, then reloaded — the marker's prefix
        // changes, so a marker row appearing after this PATCH can only
        // have come from the *reloaded* handler actually running, not a
        // stale binding from before.
        std::fs::write(
            &hook_file,
            r#"onRecordAfterUpdateSuccess((e) => {
                const markers = $app.findCollectionByNameOrId("markers");
                const marker = new Record(markers, { note: "updated-v2:" + e.record.id });
                e.app.save(marker);
                e.next();
            }, "widgets");"#,
        )
        .unwrap();
        app.jsvm()
            .unwrap()
            .reload()
            .await
            .expect("reload after editing the hook file");

        let updated_again = router
            .clone()
            .oneshot(json_request(
                "PATCH",
                &format!("/api/collections/widgets/records/{id}"),
                &token,
                serde_json::json!({"name": "after-again"}),
            ))
            .await
            .unwrap();
        assert_eq!(updated_again.status(), StatusCode::OK);

        let markers_after_reload = router
            .oneshot(authed_get(
                &format!("/api/collections/markers/records?filter=note='updated-v2:{id}'"),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(markers_after_reload.status(), StatusCode::OK);
        let markers_after_reload = body_json(markers_after_reload).await;
        assert_eq!(
            markers_after_reload["items"].as_array().unwrap().len(),
            1,
            "onRecordAfterUpdateSuccess must still fire on PATCH after a reload: {markers_after_reload}"
        );
    }

    /// Root cause of the dogfooding bug report ("editing a pb_hooks/*.pb.js
    /// while --dev runs reportedly disables record-hook enforcement until
    /// restart"): unlike the tests above, which call `Runtime::reload()`
    /// directly, this drives the *actual* `--dev` mechanism a developer
    /// relies on — `spawn_watcher`'s background poll of `snapshot()`
    /// (`crates/jsvm/src/runtime.rs`), which only ever compared each
    /// file's path and mtime. Rewriting a hook file with new content that
    /// happens to land on the *same* mtime (a coarse or stalled clock
    /// tick; trivial to hit editing a short string constant, exactly this
    /// test's "v1" -> "v2" edit) is indistinguishable from "nothing
    /// changed" to that comparison, so the watcher never notices and the
    /// stale (v1) hook stays bound — indefinitely, since nothing about a
    /// *subsequent*, differently-timed edit is needed to unstick it; the
    /// process just never reloads again until something else forces it.
    /// Forces the second write's mtime to exactly equal the first's to
    /// make the collision deterministic rather than hoping to win a race
    /// against filesystem timestamp resolution.
    #[tokio::test]
    async fn editing_a_hook_file_with_no_mtime_change_still_reloads_via_the_dev_watcher() {
        let (app, _dir, hooks_dir) = dev_app_with_empty_hooks().await;
        let router = crate::router(app.clone());
        let token = superuser_token(&app).await;

        let create_collection = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/collections",
                &token,
                serde_json::json!({
                    "name": "widgets",
                    "type": "base",
                    "listRule": "",
                    "viewRule": "",
                    "createRule": "",
                    "updateRule": "",
                    "deleteRule": "",
                    "fields": [{"name": "name", "type": "text"}],
                }),
            ))
            .await
            .unwrap();
        assert_eq!(create_collection.status(), StatusCode::OK);

        let reject_message = || {
            let router = router.clone();
            let token = token.clone();
            async move {
                let resp = router
                    .oneshot(json_request(
                        "POST",
                        "/api/collections/widgets/records",
                        &token,
                        serde_json::json!({}),
                    ))
                    .await
                    .unwrap();
                if resp.status() != StatusCode::BAD_REQUEST {
                    return None;
                }
                let body = body_json(resp).await;
                body["message"].as_str().map(str::to_string)
            }
        };

        let hook_file = hooks_dir.join("main.pb.js");
        std::fs::write(
            &hook_file,
            r#"onRecordCreate((e) => {
                if (!e.record.get("name")) { throw new BadRequestError("watcher v1"); }
                e.next();
            }, "widgets");"#,
        )
        .unwrap();

        // No manual `reload()` here — only the `--dev` watcher, which
        // polls once a second, gets this first version live.
        let saw_v1 = poll_until(std::time::Duration::from_secs(5), || {
            let check = reject_message;
            async move { check().await.as_deref() == Some("watcher v1") }
        })
        .await;
        assert!(
            saw_v1,
            "the --dev watcher never picked up the first hook file write"
        );

        // Same mtime as the file already has, new content — the same
        // shape of edit a developer makes tweaking a string in place.
        let stuck_mtime = std::fs::metadata(&hook_file).unwrap().modified().unwrap();
        std::fs::write(
            &hook_file,
            r#"onRecordCreate((e) => {
                if (!e.record.get("name")) { throw new BadRequestError("watcher v2"); }
                e.next();
            }, "widgets");"#,
        )
        .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&hook_file)
            .unwrap()
            .set_modified(stuck_mtime)
            .unwrap();
        assert_eq!(
            std::fs::metadata(&hook_file).unwrap().modified().unwrap(),
            stuck_mtime,
            "test setup: the second write must keep exactly the first write's mtime"
        );

        let saw_v2 = poll_until(std::time::Duration::from_secs(5), || {
            let check = reject_message;
            async move { check().await.as_deref() == Some("watcher v2") }
        })
        .await;
        assert!(
            saw_v2,
            "a hook file rewritten with the same mtime as before must still reload via the \
             --dev watcher, not stay stuck on the previous version until something else \
             happens to change the mtime"
        );
    }
}
