//! Pool lifecycle, reload, timeout and transaction regression tests, run
//! against a minimal in-memory `HostApi`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use cratebase_core::{AppError, Collection, Record, Settings};
use cratebase_jsvm::{
    CronHandlerId, HookHandlerId, HookKind, HostApi, HttpRequest, HttpResponse, JsBody, JsRequest,
    PoolState, RouteHandlerId, Runtime, RuntimeConfig, TransactionFn,
};
use serde_json::{Map, Value};

#[derive(Default)]
struct Mock {
    me: Weak<Mock>,
    /// Chronological host-side registration events.
    events: Mutex<Vec<String>>,
    routes: Mutex<HashMap<String, String>>,
    hang_transactions: bool,
}

impl Mock {
    fn new(hang_transactions: bool) -> Arc<Mock> {
        Arc::new_cyclic(|me| Mock {
            me: me.clone(),
            hang_transactions,
            ..Default::default()
        })
    }
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
    fn route_ids(&self) -> Vec<String> {
        let mut v: Vec<_> = self.routes.lock().unwrap().values().cloned().collect();
        v.sort();
        v
    }
}

#[async_trait]
impl HostApi for Mock {
    fn settings(&self) -> Settings {
        Settings::default()
    }
    async fn find_collection(&self, _: &str) -> Result<Arc<Collection>, AppError> {
        unimplemented!()
    }
    async fn find_record_by_id(&self, _: &str, _: &str) -> Result<Record, AppError> {
        unimplemented!()
    }
    async fn find_records_by_filter(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: i64,
        _: i64,
        _: Map<String, Value>,
    ) -> Result<Vec<Record>, AppError> {
        unimplemented!()
    }
    async fn raw_query(
        &self,
        _: &str,
        _: Map<String, Value>,
    ) -> Result<Vec<Map<String, Value>>, AppError> {
        unimplemented!()
    }
    async fn db_exec(&self, _: &str, _: Map<String, Value>) -> Result<u64, AppError> {
        unimplemented!()
    }
    async fn save_record(&self, _: Record) -> Result<Record, AppError> {
        unimplemented!()
    }
    async fn delete_record(&self, _: Record) -> Result<(), AppError> {
        unimplemented!()
    }
    async fn save_collection(&self, _: Collection) -> Result<Collection, AppError> {
        unimplemented!()
    }
    async fn delete_collection(&self, _: &str) -> Result<(), AppError> {
        unimplemented!()
    }
    async fn run_in_transaction(&self, f: TransactionFn) -> Result<(), AppError> {
        let me: Arc<dyn HostApi> = self.me.upgrade().unwrap();
        let r = f(me).await;
        if self.hang_transactions {
            // A host task that never finishes committing.
            std::future::pending::<()>().await;
        }
        r
    }
    async fn send_mail(
        &self,
        _: Vec<(String, String)>,
        _: String,
        _: String,
    ) -> Result<(), AppError> {
        unimplemented!()
    }
    async fn mails_send(&self, _: Map<String, Value>) -> Result<Value, AppError> {
        unimplemented!()
    }
    async fn notify_send(&self, _: Map<String, Value>) -> Result<Value, AppError> {
        unimplemented!()
    }
    async fn realtime_publish(&self, _: String, _: String, _: Value) -> Result<(), AppError> {
        unimplemented!()
    }
    async fn http_send(&self, _: HttpRequest) -> Result<HttpResponse, AppError> {
        unimplemented!()
    }
    fn log(&self, _: i32, _: &str, _: Value) {}
    fn store_get(&self, _: &str) -> Option<Value> {
        None
    }
    fn store_set(&self, _: &str, _: Value) {}
    fn register_route(&self, method: &str, path: &str, handler: RouteHandlerId) {
        self.events
            .lock()
            .unwrap()
            .push(format!("route {method} {path}"));
        self.routes
            .lock()
            .unwrap()
            .insert(handler.0.clone(), handler.0);
    }
    fn register_cron(&self, id: &str, _: &str, _: CronHandlerId) {
        self.events.lock().unwrap().push(format!("cron {id}"));
    }
    fn remove_cron(&self, id: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("remove_cron {id}"));
    }
    fn clear_routes(&self) {
        self.events.lock().unwrap().push("clear_routes".into());
        self.routes.lock().unwrap().clear();
    }
    fn register_hook(&self, _: HookKind, _: Vec<String>, handler: HookHandlerId, _: i32) {
        self.events
            .lock()
            .unwrap()
            .push(format!("hook {}", handler.0));
    }
    fn unregister_hook(&self, id: &str) {
        self.events.lock().unwrap().push(format!("unhook {id}"));
    }
}

fn cfg(dir: &Path, workers: usize) -> RuntimeConfig {
    let mut c = RuntimeConfig::new(dir.join("cb_hooks"), dir.join("cb_migrations"));
    c.workers = workers;
    c.timeout = Duration::from_secs(5);
    c
}

fn write(dir: &Path, name: &str, src: &str) {
    let hooks = dir.join("cb_hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join(name), src).unwrap();
}

fn route_src(value: u32) -> String {
    format!(r#"routerAdd("GET", "/a", (e) => e.json(200, {{ v: {value} }}));"#)
}

async fn call(rt: &Runtime, host: &Mock) -> Result<Value, AppError> {
    let id = host.route_ids().into_iter().next().expect("a route");
    let resp = rt
        .call_route(
            &RouteHandlerId(id),
            JsRequest {
                method: "GET".into(),
                path: "/a".into(),
                ..Default::default()
            },
        )
        .await?;
    match resp.body {
        JsBody::Json(v) => Ok(v),
        other => panic!("unexpected body {other:?}"),
    }
}

#[tokio::test]
async fn failed_reload_keeps_previous_handlers_and_marks_degraded() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.cb.js", &route_src(1));
    let host = Mock::new(false);
    let rt = Runtime::start(host.clone(), cfg(dir.path(), 3))
        .await
        .unwrap();
    assert_eq!(rt.status().state, PoolState::Ready);
    assert_eq!(rt.status().generation, 1);
    let clears_before = host
        .events()
        .iter()
        .filter(|e| *e == "clear_routes")
        .count();

    write(dir.path(), "a.cb.js", "routerAdd('GET', '/a', ((");
    assert!(rt.reload().await.is_err());
    let st = rt.status();
    assert_eq!(st.state, PoolState::Degraded);
    assert_eq!(st.generation, 1);
    assert!(st.last_error.is_some());
    // The host never lost its registrations and every worker still serves
    // the old handler.
    let clears_after = host
        .events()
        .iter()
        .filter(|e| *e == "clear_routes")
        .count();
    assert_eq!(clears_before, clears_after);
    assert_eq!(host.route_ids().len(), 1);
    for _ in 0..12 {
        assert_eq!(call(&rt, &host).await.unwrap()["v"], 1);
    }

    // A later good reload recovers, and every worker moved together.
    write(dir.path(), "a.cb.js", &route_src(2));
    rt.reload().await.unwrap();
    let st = rt.status();
    assert_eq!((st.state, st.generation), (PoolState::Ready, 2));
    assert!(st.last_error.is_none());
    for _ in 0..12 {
        assert_eq!(call(&rt, &host).await.unwrap()["v"], 2);
    }
    rt.stop();
}

#[tokio::test]
async fn failed_start_registers_nothing_with_the_host() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.cb.js", &route_src(1));
    write(dir.path(), "b.cb.js", "throw new Error('boom');");
    let host = Mock::new(false);
    assert!(Runtime::start(host.clone(), cfg(dir.path(), 3))
        .await
        .is_err());
    assert!(host.events().is_empty(), "{:?}", host.events());
}

#[tokio::test]
async fn concurrent_reloads_are_coalesced() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.cb.js", &route_src(1));
    let host = Mock::new(false);
    let rt = Runtime::start(host.clone(), cfg(dir.path(), 2))
        .await
        .unwrap();
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let rt = rt.clone();
            tokio::spawn(async move { rt.reload().await })
        })
        .collect();
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    assert!(rt.status().generation <= 3, "{:?}", rt.status());
    rt.stop();
}

#[tokio::test]
async fn stop_fails_queued_jobs_with_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.cb.js",
        r#"routerAdd("GET", "/a", (e) => { sleep(300); e.json(200, {}); });"#,
    );
    let host = Mock::new(false);
    let rt = Runtime::start(host.clone(), cfg(dir.path(), 1))
        .await
        .unwrap();
    let id = RouteHandlerId(host.route_ids().remove(0));
    let spawn_call = |rt: &Runtime, id: RouteHandlerId| {
        let rt = rt.clone();
        tokio::spawn(async move { rt.call_route(&id, JsRequest::default()).await })
    };
    let first = spawn_call(&rt, id.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued = spawn_call(&rt, id);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let rt2 = rt.clone();
    tokio::task::spawn_blocking(move || rt2.stop())
        .await
        .unwrap();
    first.await.unwrap().unwrap();
    let err = queued.await.unwrap().unwrap_err().to_string();
    assert!(err.contains("shutting down"), "{err}");
    assert_eq!(rt.status().state, PoolState::Stopped);
}

#[tokio::test]
async fn next_is_idempotent_in_a_route_chain() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.cb.js",
        r#"
        globalThis.hits = 0;
        const twice = (e) => { e.next(); e.next(); };
        routerAdd("GET", "/a", (e) => { globalThis.hits++; e.json(200, { hits: globalThis.hits }); }, twice);
        "#,
    );
    let host = Mock::new(false);
    let rt = Runtime::start(host.clone(), cfg(dir.path(), 1))
        .await
        .unwrap();
    assert_eq!(call(&rt, &host).await.unwrap()["hits"], 1);
    rt.stop();
}

#[tokio::test]
async fn timeout_is_recorded_not_rederived() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.cb.js",
        r#"
        routerAdd("GET", "/a", (e) => { while (true) {} });
        routerAdd("GET", "/b", (e) => { throw new Error("plain failure"); });
        "#,
    );
    let host = Mock::new(false);
    let mut c = cfg(dir.path(), 1);
    c.timeout = Duration::from_millis(200);
    let rt = Runtime::start(host.clone(), c).await.unwrap();
    let mut ids = host.route_ids();
    ids.sort();
    let run = |id: String| {
        let rt = rt.clone();
        async move {
            rt.call_route(&RouteHandlerId(id), JsRequest::default())
                .await
        }
    };
    let slow = ids.iter().find(|i| i.contains("GET /a")).unwrap().clone();
    let bad = ids.iter().find(|i| i.contains("GET /b")).unwrap().clone();
    let err = run(slow).await.unwrap_err().to_string();
    assert!(err.contains("timeout"), "{err}");
    let err = run(bad).await.unwrap_err().to_string();
    assert!(!err.contains("timeout"), "{err}");
    rt.stop();
}

#[tokio::test]
async fn hung_transaction_does_not_block_the_worker_forever() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.cb.js",
        r#"
        routerAdd("GET", "/a", (e) => { $app.runInTransaction(() => {}); e.json(200, {}); });
        routerAdd("GET", "/b", (e) => e.json(200, { ok: true }));
        "#,
    );
    let host = Mock::new(true);
    let mut c = cfg(dir.path(), 1);
    c.timeout = Duration::from_millis(300);
    let rt = Runtime::start(host.clone(), c).await.unwrap();
    let ids = host.route_ids();
    let a = ids.iter().find(|i| i.contains("GET /a")).unwrap().clone();
    let b = ids.iter().find(|i| i.contains("GET /b")).unwrap().clone();
    let res = tokio::time::timeout(
        Duration::from_secs(10),
        rt.call_route(&RouteHandlerId(a), JsRequest::default()),
    )
    .await
    .expect("worker must not hang on the transaction");
    assert!(res.is_err());
    // The worker is still alive and its host stack is clean.
    rt.call_route(&RouteHandlerId(b), JsRequest::default())
        .await
        .unwrap();
    rt.stop();
}
