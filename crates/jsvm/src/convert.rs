//! Conversions between `serde_json::Value` and QuickJS values, and from
//! thrown JavaScript values to [`AppError`].
//!
//! Values cross the boundary as JSON text: it is the simplest correct
//! approach (QuickJS's own `JSON.parse`/`JSON.stringify`), and the
//! payloads (a record, an event snapshot) are small.

use std::collections::BTreeMap;
use std::time::Duration;

use cratebase_core::{AppError, FieldError};
use rquickjs::{Ctx, Exception, Function, Object, Value as JsValue};
use serde_json::Value;

/// Convert a JSON value into a JavaScript value.
pub fn to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::Null => Ok(JsValue::new_null(ctx.clone())),
        Value::Bool(b) => Ok(JsValue::new_bool(ctx.clone(), *b)),
        Value::Number(n) => Ok(match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => JsValue::new_int(ctx.clone(), i as i32),
            _ => JsValue::new_float(ctx.clone(), n.as_f64().unwrap_or(0.0)),
        }),
        Value::String(s) => rquickjs::String::from_str(ctx.clone(), s).map(|s| s.into_value()),
        _ => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
            ctx.json_parse(text)
        }
    }
}

/// Convert a JavaScript value into a JSON value. `undefined`, functions
/// and symbols become `null`.
pub fn from_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Value> {
    if value.is_undefined() || value.is_null() {
        return Ok(Value::Null);
    }
    if let Some(b) = value.as_bool() {
        return Ok(Value::Bool(b));
    }
    if let Some(i) = value.as_int() {
        return Ok(Value::from(i));
    }
    if let Some(f) = value.as_float() {
        return Ok(serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null));
    }
    if let Some(s) = value.as_string() {
        return Ok(Value::String(s.to_string()?));
    }
    match ctx.json_stringify(value)? {
        Some(text) => {
            let text = text.to_string()?;
            Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
        }
        None => Ok(Value::Null),
    }
}

/// Throw an [`AppError`] into JavaScript as an `ApiError` instance (or
/// one of its status-specific subclasses) built by the prelude.
pub fn throw_app_error<'js>(ctx: &Ctx<'js>, err: &AppError) -> rquickjs::Error {
    let status = err.status();
    let message = err.to_string();
    let data: Value = match err {
        AppError::Validation { fields, .. } => serde_json::to_value(fields).unwrap_or(Value::Null),
        _ => Value::Object(Default::default()),
    };
    let make: rquickjs::Result<Function> = ctx.globals().get("__cbMakeError");
    let value = make.and_then(|make| {
        let data = to_js(ctx, &data)?;
        make.call::<_, JsValue>((status as i32, message.as_str(), data))
    });
    match value {
        Ok(v) => ctx.throw(v),
        Err(_) => Exception::throw_message(ctx, &message),
    }
}

/// Map a caught JavaScript exception to an [`AppError`].
///
/// - `ApiError` instances (and `BadRequestError` etc.) keep their status
///   and `data`.
/// - An interrupted execution (`timed_out`) becomes an internal error.
/// - Anything else is a 400 with the error's message.
pub fn js_error_to_app_error<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    timed_out: bool,
    timeout: Duration,
) -> AppError {
    if let Some(obj) = value.as_object() {
        if obj.get::<_, bool>("isApiError").unwrap_or(false) {
            return api_error_from_object(ctx, obj);
        }
    }
    let message = describe(ctx, &value);
    if timed_out || message.contains("interrupted") {
        return AppError::internal(format!(
            "JavaScript handler exceeded the {}ms timeout",
            timeout.as_millis()
        ));
    }
    if let Some(ex) = value
        .as_object()
        .and_then(|o| Exception::from_object(o.clone()))
    {
        if let Some(stack) = ex.stack() {
            tracing::debug!(stack = %stack, "JavaScript exception");
        }
    }
    AppError::bad_request(message)
}

fn api_error_from_object<'js>(ctx: &Ctx<'js>, obj: &Object<'js>) -> AppError {
    let status = obj.get::<_, i32>("status").unwrap_or(400);
    let message = obj.get::<_, String>("message").unwrap_or_default();
    let data = obj
        .get::<_, JsValue>("data")
        .ok()
        .and_then(|v| from_js(ctx, v).ok())
        .unwrap_or(Value::Null);
    match status {
        404 => AppError::not_found(message),
        401 => AppError::unauthorized(message),
        403 => AppError::forbidden(message),
        429 => AppError::TooManyRequests(if message.is_empty() {
            AppError::DEFAULT_TOO_MANY.into()
        } else {
            message
        }),
        500..=599 => AppError::internal(message),
        _ => {
            let fields = validation_fields(&data);
            if fields.is_empty() {
                AppError::bad_request(message)
            } else {
                AppError::validation(
                    if message.is_empty() {
                        AppError::DEFAULT_BAD_REQUEST.to_string()
                    } else {
                        message
                    },
                    fields,
                )
            }
        }
    }
}

/// Interpret `data` as PocketBase's `{field: {code, message}}` map, or as
/// `{field: "message"}` shorthand.
fn validation_fields(data: &Value) -> BTreeMap<String, FieldError> {
    let mut out = BTreeMap::new();
    let Value::Object(map) = data else {
        return out;
    };
    for (k, v) in map {
        match v {
            Value::Object(inner) => {
                let code = inner
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("validation_invalid_value");
                let message = inner
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Invalid value.");
                out.insert(k.clone(), FieldError::new(code, message));
            }
            Value::String(s) => {
                out.insert(k.clone(), FieldError::new("validation_invalid_value", s));
            }
            _ => {}
        }
    }
    out
}

/// A human-readable description of a thrown value.
fn describe<'js>(ctx: &Ctx<'js>, value: &JsValue<'js>) -> String {
    if let Some(obj) = value.as_object() {
        if let Some(ex) = Exception::from_object(obj.clone()) {
            if let Some(m) = ex.message() {
                return m;
            }
        }
        if let Ok(m) = obj.get::<_, String>("message") {
            return m;
        }
    }
    if let Some(s) = value.as_string() {
        return s.to_string().unwrap_or_default();
    }
    match ctx.json_stringify(value.clone()) {
        Ok(Some(s)) => s.to_string().unwrap_or_default(),
        _ => "JavaScript error".to_string(),
    }
}
