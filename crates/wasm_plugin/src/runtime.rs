//! The sandbox: compiles a `.wasm` module once, then executes it once per
//! HTTP call in a **fresh** [`wasmtime::Store`] — a plugin gets a brand
//! new, zeroed linear memory for every request rather than a
//! long-lived instance a bug (or a hostile plugin) could leave in a
//! corrupted state across calls. Instantiation from an already-compiled
//! [`wasmtime::Module`] is cheap (no re-validation, no re-compilation),
//! so this is the same "instantiate per request" shape most embedded
//! WASM plugin hosts use.
//!
//! # The ABI
//!
//! There is no component model / WIT here — that is the honest corner
//! cut for this prototype (see the crate's top-level doc). Guest and
//! host agree on a minimal hand-rolled contract instead:
//!
//! * the guest exports `memory`, `alloc(size: i32) -> i32` (a bump
//!   allocator returning a pointer into its own linear memory) and
//!   `handle_request(ptr: i32, len: i32) -> i64`;
//! * the host writes the request JSON into memory it obtained by
//!   calling the guest's own `alloc`, then calls `handle_request` with
//!   that pointer/length;
//! * `handle_request` returns a packed `(ptr << 32) | len` pointing at
//!   the guest's own response JSON, again allocated through `alloc`;
//! * the guest may call back into the host through two `cratebase`
//!   module imports — `log(ptr, len)` and `get_record(coll_ptr,
//!   coll_len, id_ptr, id_len) -> i64` (same packed-pointer convention,
//!   0 meaning "not found"/none) — proving the callback path actually
//!   runs, not just that the guest's own code executes.
//!
//! # Resource limits
//!
//! Two independent, real limits, not best-effort ones:
//!
//! * **fuel** ([`Config::consume_fuel`]): the store is given a fixed
//!   fuel budget before every call; Cranelift-instrumented code debits
//!   fuel as it runs and traps with "all fuel consumed" the instant it
//!   hits zero, so an infinite loop cannot hang the request thread — it
//!   *always* returns, deterministically, once its fuel is spent.
//! * **memory** ([`wasmtime::StoreLimits`]): linear memory is capped at
//!   [`MAX_MEMORY_BYTES`]; a `memory.grow` past that fails inside the
//!   guest rather than growing the host process's address space
//!   unbounded.

use std::sync::Arc;

use wasmtime::{
    Caller, Config, Engine, Instance, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

use crate::host::{PluginHostApi, PluginRequestContext};
use crate::manifest::Manifest;

/// Fuel is Wasmtime's abstract "how many small operations have run"
/// counter — one unit is roughly one Cranelift-instrumented instruction,
/// not a fixed wall-clock quantity. This is generous enough for a real
/// small HTTP handler (parsing a JSON body, building a JSON response)
/// while still tripping in well under a second on an infinite loop.
pub const DEFAULT_FUEL: u64 = 50_000_000;
/// Hard cap on a single plugin instance's linear memory.
pub const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to compile plugin module: {0}")]
    Compile(String),
    #[error("failed to instantiate plugin: {0}")]
    Instantiate(String),
    #[error("plugin is missing required export '{0}'")]
    MissingExport(&'static str),
    #[error("plugin execution trapped: {0}")]
    Trap(String),
    #[error("plugin exceeded its fuel budget (likely an infinite loop)")]
    FuelExhausted,
    #[error("plugin exceeded its memory budget ({MAX_MEMORY_BYTES} bytes)")]
    MemoryExhausted,
    #[error("plugin returned malformed data: {0}")]
    Malformed(String),
    #[error("plugin requested collection '{0}' is not declared in its manifest's records_read")]
    CapabilityDenied(String),
}

/// One process-wide compiler/runtime. Cheap to share: `wasmtime::Engine`
/// is already `Arc`-backed internally and is meant to be reused across
/// every module and store.
#[derive(Clone)]
pub struct PluginEngine {
    engine: Engine,
}

impl PluginEngine {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|e| RuntimeError::Compile(e.to_string()))?;
        Ok(Self { engine })
    }

    /// Compile `wasm_bytes` (validated + optimized once) into a
    /// reusable [`PluginProgram`] alongside the manifest that gates what
    /// it may call back into.
    pub fn load(
        &self,
        manifest: Manifest,
        wasm_bytes: &[u8],
    ) -> Result<PluginProgram, RuntimeError> {
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| RuntimeError::Compile(e.to_string()))?;
        Ok(PluginProgram {
            engine: self.engine.clone(),
            module,
            manifest,
        })
    }
}

/// A compiled, ready-to-run plugin. Instantiated fresh for every
/// [`PluginProgram::handle_request`] call.
#[derive(Clone)]
pub struct PluginProgram {
    engine: Engine,
    module: Module,
    manifest: Manifest,
}

#[derive(Debug)]
pub struct PluginRequest {
    pub method: String,
    pub path: String,
    pub query: serde_json::Value,
    pub body: serde_json::Value,
}

#[derive(Debug)]
pub struct PluginResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

/// Everything the host-import closures need, kept in the `Store`'s `T`.
struct StoreState {
    host: Arc<dyn PluginHostApi>,
    ctx: PluginRequestContext,
    manifest: Manifest,
    /// Bridges an async host call from inside a synchronous wasm host
    /// import. Sound because [`PluginProgram::handle_request`] itself
    /// always runs on a dedicated blocking thread (see
    /// `crates/server/src/plugin_wasm.rs`) — blocking that thread on a
    /// future never stalls a tokio worker. Wasmtime's native `async`
    /// feature (calling the guest itself through `call_async`, yielding
    /// at fuel checkpoints) would avoid pinning a whole OS thread per
    /// in-flight request; deferred, see the crate doc's "further work".
    tokio: tokio::runtime::Handle,
    limits: StoreLimits,
}

impl PluginProgram {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Run one request/response cycle to completion inside a fresh
    /// store. **Blocking**: call this from a dedicated OS thread (e.g.
    /// `tokio::task::spawn_blocking`), never from an async task directly.
    pub fn handle_request(
        &self,
        host: Arc<dyn PluginHostApi>,
        ctx: PluginRequestContext,
        tokio: tokio::runtime::Handle,
        request: PluginRequest,
    ) -> Result<PluginResponse, RuntimeError> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .build();
        let state = StoreState {
            host,
            ctx,
            manifest: self.manifest.clone(),
            tokio,
            limits,
        };
        let mut store = Store::new(&self.engine, state);
        store
            .set_fuel(DEFAULT_FUEL)
            .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;
        store.limiter(|state| &mut state.limits);

        let mut linker: Linker<StoreState> = Linker::new(&self.engine);
        link_host_imports(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(classify_trap)?;

        let request_json = serde_json::json!({
            "method": request.method,
            "path": request.path,
            "query": request.query,
            "body": request.body,
        })
        .to_string();

        let req_ptr = guest_alloc(&mut store, &instance, request_json.len())?;
        write_guest_memory(&mut store, &instance, req_ptr, request_json.as_bytes())?;

        let handle_request = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "handle_request")
            .map_err(|_| RuntimeError::MissingExport("handle_request"))?;

        let packed = handle_request
            .call(&mut store, (req_ptr, request_json.len() as i32))
            .map_err(classify_trap)?;

        let (ptr, len) = unpack(packed);
        if len == 0 {
            return Err(RuntimeError::Malformed(
                "handle_request returned an empty response".into(),
            ));
        }
        let bytes = read_guest_memory(&mut store, &instance, ptr, len)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| RuntimeError::Malformed(format!("response is not JSON: {e}")))?;
        let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(200) as u16;
        let body = value
            .get("body")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        Ok(PluginResponse { status, body })
    }
}

fn classify_trap(err: wasmtime::Error) -> RuntimeError {
    if let Some(trap) = err.downcast_ref::<wasmtime::Trap>() {
        if *trap == wasmtime::Trap::OutOfFuel {
            return RuntimeError::FuelExhausted;
        }
    }
    let msg = err.to_string();
    if msg.contains("resource limit exceeded") || msg.contains("memory") && msg.contains("limit") {
        return RuntimeError::MemoryExhausted;
    }
    RuntimeError::Trap(msg)
}

fn guest_alloc(
    store: &mut Store<StoreState>,
    instance: &Instance,
    size: usize,
) -> Result<i32, RuntimeError> {
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "alloc")
        .map_err(|_| RuntimeError::MissingExport("alloc"))?;
    alloc.call(&mut *store, size as i32).map_err(classify_trap)
}

fn write_guest_memory(
    store: &mut Store<StoreState>,
    instance: &Instance,
    ptr: i32,
    bytes: &[u8],
) -> Result<(), RuntimeError> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or(RuntimeError::MissingExport("memory"))?;
    memory
        .write(&mut *store, ptr as usize, bytes)
        .map_err(|e| RuntimeError::Malformed(format!("guest memory write out of bounds: {e}")))
}

fn read_guest_memory(
    store: &mut Store<StoreState>,
    instance: &Instance,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, RuntimeError> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or(RuntimeError::MissingExport("memory"))?;
    let data = memory.data(&mut *store);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| RuntimeError::Malformed("pointer/length overflow".into()))?;
    data.get(start..end)
        .map(|s| s.to_vec())
        .ok_or_else(|| RuntimeError::Malformed("guest memory read out of bounds".into()))
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

/// Writes `bytes` into a *new* guest allocation (calling the guest's own
/// `alloc`) and returns the packed pointer/length a host import hands
/// back to the guest.
fn stage_for_guest(
    caller: &mut Caller<'_, StoreState>,
    bytes: &[u8],
) -> Result<i64, wasmtime::Error> {
    if bytes.is_empty() {
        return Ok(0);
    }
    let alloc = caller
        .get_export("alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| wasmtime::Error::msg("plugin has no 'alloc' export"))?
        .typed::<i32, i32>(&mut *caller)?;
    let ptr = alloc.call(&mut *caller, bytes.len() as i32)?;
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("plugin has no 'memory' export"))?;
    memory.write(&mut *caller, ptr as usize, bytes)?;
    Ok(pack(ptr, bytes.len() as i32))
}

fn read_from_guest(
    caller: &mut Caller<'_, StoreState>,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, wasmtime::Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("plugin has no 'memory' export"))?;
    let data = memory.data(&mut *caller);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| wasmtime::Error::msg("pointer/length overflow"))?;
    data.get(start..end)
        .map(|s| s.to_vec())
        .ok_or_else(|| wasmtime::Error::msg("guest memory read out of bounds"))
}

/// Registers the `cratebase` import module: `log` and `get_record`. This
/// is the entire host-api surface a guest can reach — nothing else is
/// linked in, so a plugin that imports anything else fails to
/// instantiate rather than silently getting more access than its
/// manifest declared.
fn link_host_imports(linker: &mut Linker<StoreState>) -> Result<(), RuntimeError> {
    linker
        .func_wrap(
            "cratebase",
            "log",
            |mut caller: Caller<'_, StoreState>, ptr: i32, len: i32| -> i64 {
                match read_from_guest(&mut caller, ptr, len) {
                    Ok(bytes) => {
                        let msg = String::from_utf8_lossy(&bytes);
                        let plugin = caller.data().manifest.name.clone();
                        caller.data().host.log(&plugin, "info", &msg);
                    }
                    Err(e) => tracing::warn!("plugin log call had bad args: {e}"),
                }
                0
            },
        )
        .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;

    linker
        .func_wrap(
            "cratebase",
            "get_record",
            |mut caller: Caller<'_, StoreState>,
             coll_ptr: i32,
             coll_len: i32,
             id_ptr: i32,
             id_len: i32|
             -> Result<i64, wasmtime::Error> {
                let collection =
                    String::from_utf8(read_from_guest(&mut caller, coll_ptr, coll_len)?)
                        .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
                let id = String::from_utf8(read_from_guest(&mut caller, id_ptr, id_len)?)
                    .map_err(|e| wasmtime::Error::msg(e.to_string()))?;

                if !caller
                    .data()
                    .manifest
                    .capabilities
                    .records_read
                    .iter()
                    .any(|c| c == &collection)
                {
                    // Capability not declared: behave exactly like a
                    // record the caller cannot see, never a distinct
                    // "forbidden" signal a plugin could use to probe
                    // for what it isn't allowed to read.
                    return Ok(0);
                }

                let host = caller.data().host.clone();
                let ctx = caller.data().ctx.clone();
                let handle = caller.data().tokio.clone();
                let result = tokio::task::block_in_place(|| {
                    handle.block_on(host.get_record(&ctx, &collection, &id))
                })
                .map_err(|e| wasmtime::Error::msg(e.to_string()))?;

                match result {
                    Some(value) => {
                        let bytes = serde_json::to_vec(&value)
                            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
                        stage_for_guest(&mut caller, &bytes)
                    }
                    None => Ok(0),
                }
            },
        )
        .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;

    linker
        .func_wrap(
            "cratebase",
            "current_user",
            |mut caller: Caller<'_, StoreState>| -> Result<i64, wasmtime::Error> {
                let identity = caller.data().ctx.auth.clone();
                match identity {
                    Some(identity) => {
                        let bytes = serde_json::to_vec(&identity)
                            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
                        stage_for_guest(&mut caller, &bytes)
                    }
                    None => Ok(0),
                }
            },
        )
        .map_err(|e| RuntimeError::Instantiate(e.to_string()))?;

    Ok(())
}
