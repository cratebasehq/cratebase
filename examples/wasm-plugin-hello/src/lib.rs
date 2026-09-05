//! Minimal example WASM plugin for Cratebase's plugin foundation
//! (`crates/wasm_plugin`). Proves the whole pipeline end to end: the
//! host loads this `.wasm`, calls `handle_request`, and this code calls
//! *back* into the host through the `cratebase` import module to read
//! who the caller is and — for `/record` — a real, rule-enforced record.
//!
//! See `crates/wasm_plugin/src/runtime.rs`'s module doc for the exact
//! ABI this implements: `alloc`/`memory`/`handle_request` exports, and
//! `log`/`current_user`/`get_record` imports.

use serde_json::{json, Value};

#[link(wasm_import_module = "cratebase")]
unsafe extern "C" {
    fn log(ptr: i32, len: i32) -> i64;
    fn current_user() -> i64;
    fn get_record(coll_ptr: i32, coll_len: i32, id_ptr: i32, id_len: i32) -> i64;
}

/// Bump-allocates `size` bytes in this module's own linear memory and
/// hands the pointer to the host, which writes the request JSON there
/// before calling `handle_request`.
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let mut buf = Vec::<u8>::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as i32
}

fn pack(ptr: i32, len: i32) -> i64 {
    ((ptr as u32 as i64) << 32) | (len as u32 as i64)
}

fn unpack(packed: i64) -> (i32, i32) {
    (
        ((packed >> 32) & 0xFFFF_FFFF) as i32,
        (packed & 0xFFFF_FFFF) as i32,
    )
}

unsafe fn read_at(ptr: i32, len: i32) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec() }
}

/// Calls the `current_user` host import and parses whatever JSON (or
/// nothing, for an anonymous caller) it wrote into our memory.
fn fetch_current_user() -> Option<Value> {
    let packed = unsafe { current_user() };
    let (ptr, len) = unpack(packed);
    if len == 0 {
        return None;
    }
    let bytes = unsafe { read_at(ptr, len) };
    serde_json::from_slice(&bytes).ok()
}

/// Calls the `get_record` host import — the same rule-enforced path
/// `GET /api/collections/{c}/records/{id}` uses — for whatever
/// collection this plugin's manifest declared under
/// `capabilities.records_read`.
fn fetch_record(collection: &str, id: &str) -> Option<Value> {
    let coll_bytes = collection.as_bytes();
    let id_bytes = id.as_bytes();
    let packed = unsafe {
        get_record(
            coll_bytes.as_ptr() as i32,
            coll_bytes.len() as i32,
            id_bytes.as_ptr() as i32,
            id_bytes.len() as i32,
        )
    };
    let (ptr, len) = unpack(packed);
    if len == 0 {
        return None;
    }
    let bytes = unsafe { read_at(ptr, len) };
    serde_json::from_slice(&bytes).ok()
}

fn log_line(msg: &str) {
    unsafe {
        log(msg.as_ptr() as i32, msg.len() as i32);
    }
}

/// The single entry point every request the host mounted this plugin's
/// routes on comes through. `ptr`/`len` point at the request JSON
/// (`{"method", "path", "query", "body"}`) the host already wrote into
/// our memory via `alloc`; the return value is a packed pointer/length
/// of our own response JSON (`{"status", "body"}`), also in our memory.
#[no_mangle]
pub extern "C" fn handle_request(ptr: i32, len: i32) -> i64 {
    let bytes = unsafe { read_at(ptr, len) };
    let request: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let path = request.get("path").and_then(Value::as_str).unwrap_or("");

    log_line(&format!("wasm-plugin-hello handling {path}"));

    let response = if path.trim_end_matches('/').ends_with("/record") {
        let query = request.get("query").cloned().unwrap_or(Value::Null);
        let collection = query
            .get("collection")
            .and_then(Value::as_str)
            .unwrap_or("");
        let id = query.get("id").and_then(Value::as_str).unwrap_or("");
        match fetch_record(collection, id) {
            Some(record) => json!({"status": 200, "body": {"record": record}}),
            None => json!({"status": 404, "body": {"error": "record not found or not visible"}}),
        }
    } else {
        let user = fetch_current_user();
        json!({
            "status": 200,
            "body": {
                "message": "pong",
                "authenticated": user.is_some(),
                "userId": user.as_ref().and_then(|u| u.get("id")).cloned(),
            }
        })
    };

    let out = serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec());
    let out_ptr = alloc(out.len() as i32);
    unsafe {
        std::ptr::copy_nonoverlapping(out.as_ptr(), out_ptr as *mut u8, out.len());
    }
    pack(out_ptr, out.len() as i32)
}
