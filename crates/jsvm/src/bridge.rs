//! The native side of the JavaScript globals.
//!
//! The prelude (`prelude.js`) implements every PocketBase global in
//! JavaScript on top of a single native entry point, `__cb.native(op,
//! args)`, plus `__cb.compile(path, source)` for CommonJS modules. Each
//! `op` is dispatched here to the [`HostApi`] (blocking the worker thread
//! on the host future) or to a local helper (`$security`, `$os`,
//! `require`). Arguments and results cross the boundary as JSON.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cratebase_core::{AppError, Collection, Record};
use rquickjs::{Ctx, Function, Object, Value as JsValue};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::convert::{from_js, throw_app_error, to_js};
use crate::host::{
    CronHandlerId, HookHandlerId, HookKind, HostApi, HttpRequest, RecordTokenKind, RouteHandlerId,
};
use crate::runtime::JS_RECORD_OPTIONS;
use crate::security::{self, HmacAlg};
use crate::worker::{eval_options, PendingTx, WorkerState};

const PRELUDE: &str = include_str!("prelude.js");

/// Install `__cb` and evaluate the prelude, which defines every global.
pub(crate) fn install(ctx: &Ctx<'_>, state: Rc<WorkerState>) -> rquickjs::Result<()> {
    let cb = Object::new(ctx.clone())?;
    cb.set("hooksDir", abs(&state.cfg.hooks_dir))?;
    cb.set("workerIndex", state.index as i32)?;
    cb.set("currentFile", "")?;
    cb.set(
        "hookNames",
        HookKind::ALL
            .iter()
            .map(|k| k.js_name())
            .collect::<Vec<_>>(),
    )?;

    let native_state = state.clone();
    cb.set(
        "native",
        Function::new(
            ctx.clone(),
            // `native_fn` is what makes this compile: `Ctx<'_>` and
            // `JsValue<'_>` in a closure signature are two *independent*
            // inferred lifetimes, and `Value<'js>` is invariant, so the
            // returned value can't be tied back to the incoming context.
            // Passing the closure through a `for<'js>`-bounded helper
            // forces one higher-ranked lifetime across all three.
            native_fn(move |ctx, op: String, args| {
                let args = match from_js(&ctx, args)? {
                    Value::Array(a) => a,
                    _ => vec![],
                };
                match dispatch(&native_state, &op, args) {
                    Ok(v) => to_js(&ctx, &v),
                    Err(e) => Err(throw_app_error(&ctx, &e)),
                }
            }),
        )?,
    )?;

    // A free `fn` rather than a closure, for the same reason: an item can
    // name `'js` and relate its argument and return type to it.
    cb.set("compile", Function::new(ctx.clone(), compile)?)?;

    ctx.globals().set("__cb", cb)?;
    ctx.eval_with_options::<(), _>(PRELUDE, eval_options("<prelude>", true))
}

/// Pins a closure to a single higher-ranked `'js`, so `Ctx`, the argument
/// and the return value all share one lifetime instead of three inferred
/// ones. Identity at runtime; it exists purely to give inference a
/// `for<'js>` bound to unify against.
fn native_fn<F>(f: F) -> F
where
    F: for<'js> Fn(Ctx<'js>, String, JsValue<'js>) -> rquickjs::Result<JsValue<'js>>,
{
    f
}

/// `__cb.compile(path, source)` — wraps a CommonJS module body in the
/// standard function shim and evaluates it, yielding the module function.
fn compile<'js>(ctx: Ctx<'js>, path: String, source: String) -> rquickjs::Result<JsValue<'js>> {
    let wrapped =
        format!("(function (exports, require, module, __filename, __dirname) {{{source}\n}})");
    ctx.eval_with_options(wrapped, eval_options(&path, false))
}

fn abs(p: &Path) -> String {
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    p.canonicalize().unwrap_or(p).to_string_lossy().into_owned()
}

// --- argument helpers -----------------------------------------------------

struct Args(Vec<Value>);

impl Args {
    fn get(&self, i: usize) -> &Value {
        self.0.get(i).unwrap_or(&Value::Null)
    }
    fn str(&self, i: usize) -> String {
        match self.get(i) {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
    fn i64(&self, i: usize) -> i64 {
        self.get(i)
            .as_i64()
            .or_else(|| self.get(i).as_f64().map(|f| f as i64))
            .unwrap_or(0)
    }
    fn bool(&self, i: usize) -> bool {
        self.get(i).as_bool().unwrap_or(false)
    }
    fn map(&self, i: usize) -> Map<String, Value> {
        match self.get(i) {
            Value::Object(m) => m.clone(),
            _ => Map::new(),
        }
    }
    fn parse<T: for<'de> Deserialize<'de>>(&self, i: usize, what: &str) -> Result<T, AppError> {
        serde_json::from_value(self.get(i).clone())
            .map_err(|e| AppError::bad_request(format!("invalid {what}: {e}")))
    }
}

/// The JavaScript-side description of a record (`Record.__ref()`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordRef {
    #[serde(default)]
    collection: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    is_new: bool,
    #[serde(default)]
    data: Map<String, Value>,
    #[serde(default)]
    dirty: Vec<String>,
}

fn record_json(r: &Record) -> Value {
    r.to_json(JS_RECORD_OPTIONS)
}

/// Rebuild a domain [`Record`] from the JavaScript reference: a fresh
/// record for `isNew`, otherwise the stored one with the dirty fields
/// applied.
fn record_from_ref(
    state: &WorkerState,
    host: &Arc<dyn HostApi>,
    r: &RecordRef,
) -> Result<Record, AppError> {
    if r.collection.is_empty() {
        return Err(AppError::bad_request("record has no collection"));
    }
    let collection = state.block_on(host.find_collection(&r.collection))?;
    if r.is_new {
        let mut rec = Record::new(collection);
        for (k, v) in &r.data {
            if k == "collectionId" || k == "collectionName" || k == "expand" {
                continue;
            }
            rec.set(k, v.clone());
        }
        Ok(rec)
    } else {
        let mut rec = state.block_on(host.find_record_by_id(&collection.name, &r.id))?;
        for field in &r.dirty {
            if let Some(v) = r.data.get(field) {
                rec.set(field, v.clone());
            }
        }
        Ok(rec)
    }
}

// --- dispatch --------------------------------------------------------------

pub(crate) fn dispatch(
    state: &Rc<WorkerState>,
    op: &str,
    args: Vec<Value>,
) -> Result<Value, AppError> {
    let a = Args(args);
    let host = state.current_host();
    match op {
        // ---- $app -------------------------------------------------------
        "settings" => serde_json::to_value(host.settings())
            .map_err(|e| AppError::internal(format!("settings: {e}"))),
        "findCollection" => {
            let c = state.block_on(host.find_collection(&a.str(0)))?;
            Ok(c.to_json())
        }
        "findRecordById" => {
            let r = state.block_on(host.find_record_by_id(&a.str(0), &a.str(1)))?;
            Ok(record_json(&r))
        }
        "findRecordsByFilter" => {
            let rows = state.block_on(host.find_records_by_filter(
                &a.str(0),
                &a.str(1),
                &a.str(2),
                a.i64(3),
                a.i64(4),
                a.map(5),
            ))?;
            Ok(Value::Array(rows.iter().map(record_json).collect()))
        }
        "saveRecord" => {
            let r: RecordRef = a.parse(0, "record")?;
            let rec = record_from_ref(state, &host, &r)?;
            let saved = state.block_on(host.save_record(rec))?;
            Ok(record_json(&saved))
        }
        "deleteRecord" => {
            let r: RecordRef = a.parse(0, "record")?;
            let rec = record_from_ref(state, &host, &r)?;
            state.block_on(host.delete_record(rec))?;
            Ok(Value::Null)
        }
        "saveCollection" => {
            let c: Collection = a.parse(0, "collection")?;
            let saved = state.block_on(host.save_collection(c))?;
            Ok(saved.to_json())
        }
        "deleteCollection" => {
            state.block_on(host.delete_collection(&a.str(0)))?;
            Ok(Value::Null)
        }
        "txBegin" => tx_begin(state).map(|_| Value::Null),
        "txEnd" => {
            let result = if a.bool(0) {
                Ok(())
            } else {
                Err(AppError::bad_request(a.str(1)))
            };
            tx_end(state, result).map(|_| Value::Null)
        }
        "sendMail" => {
            let msg = a.map(0);
            let to = mail_recipients(msg.get("to"));
            let subject = msg
                .get("subject")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let html = msg
                .get("html")
                .or(msg.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            state.block_on(host.send_mail(to, subject, html))?;
            Ok(Value::Null)
        }
        "log" => {
            host.log(a.i64(0) as i32, &a.str(1), a.get(2).clone());
            Ok(Value::Null)
        }
        "storeGet" => Ok(host.store_get(&a.str(0)).unwrap_or(Value::Null)),
        "storeHas" => Ok(Value::Bool(host.store_get(&a.str(0)).is_some())),
        "storeSet" => {
            host.store_set(&a.str(0), a.get(1).clone());
            Ok(Value::Null)
        }

        // ---- registrations (worker 0 reports) --------------------------
        "registerHook" => {
            if state.reports() {
                let id = a.str(0);
                let kind = HookKind::from_js_name(&a.str(1))
                    .ok_or_else(|| AppError::bad_request(format!("unknown hook {}", a.str(1))))?;
                let tags: Vec<String> = a.parse(2, "tags").unwrap_or_default();
                let priority = a.i64(3) as i32;
                state.registered_hooks.borrow_mut().push(id.clone());
                host.register_hook(kind, tags, HookHandlerId(id), priority);
            }
            Ok(Value::Null)
        }
        "registerRoute" => {
            if state.reports() {
                host.register_route(&a.str(1), &a.str(2), RouteHandlerId(a.str(0)));
            }
            Ok(Value::Null)
        }
        "registerCron" => {
            if state.reports() {
                let id = a.str(0);
                state.registered_crons.borrow_mut().push(id.clone());
                host.register_cron(&id, &a.str(1), CronHandlerId(id.clone()));
            }
            Ok(Value::Null)
        }
        "removeCron" => {
            if state.reports() {
                let id = a.str(0);
                state.registered_crons.borrow_mut().retain(|c| c != &id);
                host.remove_cron(&id);
            }
            Ok(Value::Null)
        }

        // ---- $http --------------------------------------------------------
        "httpSend" => {
            let cfg = a.map(0);
            let req = HttpRequest {
                method: cfg
                    .get("method")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .unwrap_or("GET")
                    .to_uppercase(),
                url: cfg
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                headers: cfg
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|h| {
                        h.iter()
                            .map(|(k, v)| (k.clone(), value_to_string(v)))
                            .collect()
                    })
                    .unwrap_or_default(),
                body: match cfg.get("body") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) => Some(s.clone().into_bytes()),
                    Some(Value::Array(items)) if items.iter().all(Value::is_number) => Some(
                        items
                            .iter()
                            .filter_map(Value::as_u64)
                            .map(|b| b as u8)
                            .collect(),
                    ),
                    Some(other) => Some(serde_json::to_vec(other).unwrap_or_default()),
                },
                timeout_secs: cfg.get("timeout").and_then(Value::as_u64).unwrap_or(0),
            };
            let resp = state.block_on(host.http_send(req))?;
            let raw = String::from_utf8_lossy(&resp.body).into_owned();
            let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            Ok(json!({
                "statusCode": resp.status,
                "headers": resp.headers,
                "raw": raw,
                "json": parsed,
            }))
        }

        // ---- misc ---------------------------------------------------------
        "sleep" => {
            let ms = a.i64(0).max(0) as u64;
            std::thread::sleep(Duration::from_millis(ms));
            Ok(Value::Null)
        }
        "loadModule" => {
            let from = PathBuf::from(a.str(0));
            let m = crate::modules::load(&state.cfg.hooks_dir, &from, &a.str(1))?;
            serde_json::to_value(m).map_err(|e| AppError::internal(e.to_string()))
        }
        "os" => os_op(&a),
        "security" => security_op(&a),

        // ---- auth-adjacent (delegated to optional host methods) -----------
        "hashPassword" => Ok(host.hash_password(&a.str(0))),
        "validatePassword" => {
            let r: RecordRef = a.parse(0, "record")?;
            let rec = record_from_ref(state, &host, &r)?;
            let ok = state.block_on(host.validate_password(&rec, &a.str(1)))?;
            Ok(Value::Bool(ok))
        }
        "recordToken" => {
            let r: RecordRef = a.parse(0, "record")?;
            let rec = record_from_ref(state, &host, &r)?;
            let kind = token_kind(&a.str(1))?;
            let token = state.block_on(host.create_record_token(&rec, kind))?;
            Ok(Value::String(token))
        }
        "sendRecordMail" => {
            let r: RecordRef = a.parse(0, "record")?;
            let rec = record_from_ref(state, &host, &r)?;
            send_record_mail(state, &host, &rec, &a.str(1))?;
            Ok(Value::Null)
        }
        other => Err(AppError::internal(format!("unknown native op {other}"))),
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn mail_recipients(to: Option<&Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |v: &Value| match v {
        Value::String(s) => out.push((s.clone(), String::new())),
        Value::Object(m) => {
            let address = m.get("address").and_then(Value::as_str).unwrap_or("");
            let name = m.get("name").and_then(Value::as_str).unwrap_or("");
            if !address.is_empty() {
                out.push((address.to_string(), name.to_string()));
            }
        }
        _ => {}
    };
    match to {
        Some(Value::Array(items)) => items.iter().for_each(&mut push),
        Some(v) => push(v),
        None => {}
    }
    out
}

fn token_kind(name: &str) -> Result<RecordTokenKind, AppError> {
    Ok(match name {
        "auth" => RecordTokenKind::Auth,
        "verification" => RecordTokenKind::Verification,
        "passwordReset" => RecordTokenKind::PasswordReset,
        "emailChange" => RecordTokenKind::EmailChange,
        "file" => RecordTokenKind::File,
        other => return Err(AppError::bad_request(format!("unknown token kind {other}"))),
    })
}

/// `$mails.sendRecord*`: render the collection's template with the
/// PocketBase placeholders and hand it to the host mailer.
fn send_record_mail(
    state: &WorkerState,
    host: &Arc<dyn HostApi>,
    record: &Record,
    kind: &str,
) -> Result<(), AppError> {
    let collection = record.collection();
    let (template, token_kind) = match kind {
        "verification" => (
            &collection.auth.verification_template,
            Some(RecordTokenKind::Verification),
        ),
        "passwordReset" => (
            &collection.auth.reset_password_template,
            Some(RecordTokenKind::PasswordReset),
        ),
        "emailChange" => (
            &collection.auth.confirm_email_change_template,
            Some(RecordTokenKind::EmailChange),
        ),
        "otp" => (&collection.auth.otp.email_template, None),
        other => return Err(AppError::bad_request(format!("unknown mail kind {other}"))),
    };
    let token = match token_kind {
        Some(k) => match state.block_on(host.create_record_token(record, k)) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "record token unavailable; sending mail with an empty {{TOKEN}}");
                String::new()
            }
        },
        None => String::new(),
    };
    let settings = host.settings();
    let render = |text: &str| {
        let mut out = text
            .replace("{APP_NAME}", &settings.meta.app_name)
            .replace("{APP_URL}", &settings.meta.app_url)
            .replace("{TOKEN}", &token);
        for (k, v) in record.data() {
            out = out.replace(&format!("{{RECORD:{k}}}"), &value_to_string(v));
        }
        out
    };
    let email = record.email();
    if email.is_empty() {
        return Err(AppError::bad_request("record has no email"));
    }
    state.block_on(host.send_mail(
        vec![(email, record.get_string("name"))],
        render(&template.subject),
        render(&template.body),
    ))
}

fn os_op(a: &Args) -> Result<Value, AppError> {
    match a.str(0).as_str() {
        "getenv" => Ok(Value::String(std::env::var(a.str(1)).unwrap_or_default())),
        "readFile" => std::fs::read_to_string(a.str(1))
            .map(Value::String)
            .map_err(|e| AppError::bad_request(format!("readFile: {e}"))),
        "writeFile" => std::fs::write(a.str(1), a.str(2))
            .map(|_| Value::Null)
            .map_err(|e| AppError::bad_request(format!("writeFile: {e}"))),
        "exists" => Ok(Value::Bool(Path::new(&a.str(1)).exists())),
        "tempDir" => Ok(Value::String(
            std::env::temp_dir().to_string_lossy().into_owned(),
        )),
        "getwd" => Ok(Value::String(
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )),
        other => Err(AppError::bad_request(format!(
            "$os.{other} is not supported"
        ))),
    }
}

fn security_op(a: &Args) -> Result<Value, AppError> {
    let s = |v: String| Ok(Value::String(v));
    match a.str(0).as_str() {
        "randomString" => {
            let alphabet = a.str(2);
            let alphabet = if alphabet.is_empty() {
                security::DEFAULT_ALPHABET
            } else {
                alphabet.as_str()
            };
            s(security::random_string(a.i64(1).max(0) as usize, alphabet))
        }
        "sha256" => s(security::sha256(&a.str(1))),
        "sha512" => s(security::sha512(&a.str(1))),
        "sha1" => s(security::sha1(&a.str(1))),
        "md5" => s(security::md5(&a.str(1))),
        "hs256" => s(security::hmac_hex(
            HmacAlg::Hs256,
            a.str(1).as_bytes(),
            a.str(2).as_bytes(),
        )),
        "hs512" => s(security::hmac_hex(
            HmacAlg::Hs512,
            a.str(1).as_bytes(),
            a.str(2).as_bytes(),
        )),
        "createJWT" => {
            let alg = if a.str(4) == "HS512" {
                HmacAlg::Hs512
            } else {
                HmacAlg::Hs256
            };
            s(security::create_jwt(alg, &a.map(1), &a.str(2), a.i64(3))?)
        }
        "parseUnverifiedJWT" => Ok(Value::Object(security::parse_unverified_jwt(&a.str(1))?)),
        "parseJWT" => Ok(Value::Object(security::parse_jwt(&a.str(1), &a.str(2))?)),
        "encrypt" => s(security::encrypt(&a.str(1), &a.str(2))?),
        "decrypt" => s(security::decrypt(&a.str(1), &a.str(2))?),
        "equal" => Ok(Value::Bool(security::constant_time_eq(
            a.str(1).as_bytes(),
            a.str(2).as_bytes(),
        ))),
        other => Err(AppError::bad_request(format!(
            "$security.{other} is not supported"
        ))),
    }
}

// --- transactions ----------------------------------------------------------

/// Open a host transaction and make it the current host for JavaScript.
///
/// The host's `run_in_transaction` wants a `Send + 'static` closure whose
/// future it awaits inside its own task, while the JavaScript body must
/// run on *this* thread. The two are bridged with channels: the
/// transaction is spawned on the tokio runtime; its closure sends the
/// transactional host back to this thread and then waits on a `oneshot`
/// for the JavaScript body's result. This thread receives the host,
/// pushes it, and returns to JavaScript. [`tx_end`] completes the
/// `oneshot` and blocks on the task for the commit/rollback outcome.
pub(crate) fn tx_begin(state: &WorkerState) -> Result<(), AppError> {
    let host = state.current_host();
    let (host_tx, host_rx) = std::sync::mpsc::channel::<Arc<dyn HostApi>>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<(), AppError>>();
    let join = state.handle.spawn(async move {
        host.run_in_transaction(Box::new(move |tx| {
            Box::pin(async move {
                if host_tx.send(tx).is_err() {
                    return Err(AppError::internal("jsvm transaction: worker went away"));
                }
                done_rx
                    .await
                    .unwrap_or_else(|_| Err(AppError::internal("jsvm transaction abandoned")))
            })
        }))
        .await
    });
    match host_rx.recv() {
        Ok(tx) => {
            state.host_stack.borrow_mut().push(tx);
            state.tx_stack.borrow_mut().push(PendingTx {
                done: Some(done_tx),
                join,
            });
            Ok(())
        }
        Err(_) => {
            // The host failed before invoking the closure; surface its error.
            match state.handle.block_on(join) {
                Ok(Err(e)) => Err(e),
                Ok(Ok(())) => Err(AppError::internal(
                    "host did not invoke the transaction closure",
                )),
                Err(e) => Err(AppError::internal(format!("transaction task failed: {e}"))),
            }
        }
    }
}

/// Finish the innermost transaction with the JavaScript body's result and
/// return the host's commit/rollback outcome.
pub(crate) fn tx_end(state: &WorkerState, result: Result<(), AppError>) -> Result<(), AppError> {
    let Some(mut pending) = state.tx_stack.borrow_mut().pop() else {
        return Err(AppError::internal("txEnd without txBegin"));
    };
    state.host_stack.borrow_mut().pop();
    if let Some(done) = pending.done.take() {
        let _ = done.send(result);
    }
    match state.handle.block_on(pending.join) {
        Ok(r) => r,
        Err(e) => Err(AppError::internal(format!("transaction task failed: {e}"))),
    }
}

/// Roll back transactions JavaScript left open (e.g. after an interrupt).
pub(crate) fn abort_open_transactions(state: &WorkerState) {
    while !state.tx_stack.borrow().is_empty() {
        let _ = tx_end(
            state,
            Err(AppError::internal(
                "transaction aborted: handler did not finish",
            )),
        );
    }
}
