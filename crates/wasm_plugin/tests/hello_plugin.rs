//! End-to-end tests against the real `examples/wasm-plugin-hello`
//! binary (proving the ABI actually round-trips, not just that unit
//! tests of the Rust code pass) plus a synthetic infinite-loop module
//! (hand-written WAT — no toolchain dependency) proving the fuel limit
//! genuinely stops a runaway plugin rather than hanging.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cratebase_wasm_plugin::{
    AuthIdentity, Manifest, PluginEngine, PluginHostApi, PluginHostError, PluginRequest,
    PluginRequestContext, RuntimeError,
};
use serde_json::Value;

struct MockHost;

#[async_trait]
impl PluginHostApi for MockHost {
    async fn get_record(
        &self,
        _ctx: &PluginRequestContext,
        collection: &str,
        id: &str,
    ) -> Result<Option<Value>, PluginHostError> {
        if collection == "users" && id == "abc" {
            Ok(Some(serde_json::json!({"id": "abc", "name": "Ada"})))
        } else {
            Ok(None)
        }
    }

    fn log(&self, plugin: &str, level: &str, message: &str) {
        eprintln!("[{plugin}] {level}: {message}");
    }
}

fn hello_program() -> cratebase_wasm_plugin::PluginProgram {
    let wasm = include_bytes!("../../../examples/wasm-plugin-hello/plugin.wasm");
    let manifest = Manifest::parse(include_str!(
        "../../../examples/wasm-plugin-hello/plugin.toml"
    ))
    .unwrap();
    PluginEngine::new().unwrap().load(manifest, wasm).unwrap()
}

#[test]
fn ping_round_trips_through_the_current_user_host_callback() {
    let program = hello_program();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let host: Arc<dyn PluginHostApi> = Arc::new(MockHost);
    let ctx = PluginRequestContext {
        auth: Some(AuthIdentity {
            id: "u1".into(),
            collection_id: "c1".into(),
            collection_name: "users".into(),
            email: None,
        }),
    };
    let response = program
        .handle_request(
            host,
            ctx,
            rt.handle().clone(),
            PluginRequest {
                method: "GET".into(),
                path: "/api/plugins/hello/ping".into(),
                query: Value::Null,
                body: Value::Null,
            },
        )
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body["authenticated"], true);
    assert_eq!(response.body["userId"], "u1");
}

#[test]
fn ping_reports_anonymous_when_no_auth_context_is_given() {
    let program = hello_program();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let host: Arc<dyn PluginHostApi> = Arc::new(MockHost);
    let response = program
        .handle_request(
            host,
            PluginRequestContext::default(),
            rt.handle().clone(),
            PluginRequest {
                method: "GET".into(),
                path: "/api/plugins/hello/ping".into(),
                query: Value::Null,
                body: Value::Null,
            },
        )
        .unwrap();
    assert_eq!(response.body["authenticated"], false);
}

#[test]
fn record_route_reaches_the_real_get_record_host_callback() {
    let program = hello_program();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let host: Arc<dyn PluginHostApi> = Arc::new(MockHost);
    let response = program
        .handle_request(
            host,
            PluginRequestContext::default(),
            rt.handle().clone(),
            PluginRequest {
                method: "GET".into(),
                path: "/api/plugins/hello/record".into(),
                query: serde_json::json!({"collection": "users", "id": "abc"}),
                body: Value::Null,
            },
        )
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body["record"]["name"], "Ada");
}

/// A hand-written infinite-loop WASM module — no toolchain dependency
/// on the example plugin — proving fuel exhaustion, not the example
/// plugin's own good behaviour, is what stops a runaway guest.
const INFINITE_LOOP_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) i32.const 0)
  (func (export "handle_request") (param i32 i32) (result i64)
    (loop $forever
      br $forever)
    i64.const 0)
)
"#;

#[test]
fn an_infinite_loop_is_trapped_by_fuel_instead_of_hanging() {
    let wasm = wat::parse_str(INFINITE_LOOP_WAT).unwrap();
    let manifest = Manifest::parse(
        r#"
        name = "loopy"
        version = "0.1.0"
        entry = "loopy.wasm"
        [capabilities]
        routes = ["x"]
        "#,
    )
    .unwrap();
    let engine = PluginEngine::new().unwrap();
    let program = engine.load(manifest, &wasm).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let host: Arc<dyn PluginHostApi> = Arc::new(MockHost);

    let start = Instant::now();
    let result = program.handle_request(
        host,
        PluginRequestContext::default(),
        rt.handle().clone(),
        PluginRequest {
            method: "GET".into(),
            path: "/x".into(),
            query: Value::Null,
            body: Value::Null,
        },
    );
    let elapsed = start.elapsed();

    assert!(
        matches!(result, Err(RuntimeError::FuelExhausted)),
        "expected a fuel-exhaustion trap, got {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "an infinite loop must be killed quickly, took {elapsed:?}"
    );
}

/// A module whose declared *initial* memory alone (125 MiB, in 64 KiB
/// pages) already exceeds `runtime::MAX_MEMORY_BYTES` (64 MiB) must
/// fail to instantiate rather than being granted it.
#[test]
fn a_module_over_the_memory_cap_is_refused_at_instantiation() {
    let wat = r#"
    (module
      (memory (export "memory") 2000)
      (func (export "alloc") (param i32) (result i32) i32.const 0)
      (func (export "handle_request") (param i32 i32) (result i64) i64.const 0)
    )
    "#;
    let wasm = wat::parse_str(wat).unwrap();
    let manifest = Manifest::parse(
        r#"
        name = "hungry"
        version = "0.1.0"
        entry = "hungry.wasm"
        [capabilities]
        routes = ["x"]
        "#,
    )
    .unwrap();
    let engine = PluginEngine::new().unwrap();
    let program = engine.load(manifest, &wasm).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let host: Arc<dyn PluginHostApi> = Arc::new(MockHost);
    let result = program.handle_request(
        host,
        PluginRequestContext::default(),
        rt.handle().clone(),
        PluginRequest {
            method: "GET".into(),
            path: "/x".into(),
            query: Value::Null,
            body: Value::Null,
        },
    );
    assert!(
        matches!(
            result,
            Err(RuntimeError::MemoryExhausted) | Err(RuntimeError::Trap(_))
        ),
        "expected the store's memory limiter to refuse this module, got {result:?}"
    );
}
