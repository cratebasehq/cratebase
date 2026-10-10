//! Multi-project host: isolation and routing across projects in one
//! process.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use cratebase_server::host;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET_A: &str = "secret-a-0123456789-0123456789-0123456789";
const SECRET_B: &str = "secret-b-0123456789-0123456789-0123456789";

fn write_project(
    root: &std::path::Path,
    name: &str,
    hosts: &str,
    token: &str,
    secret: &str,
    mark: &str,
) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("cb_hooks")).unwrap();
    std::fs::write(
        dir.join("project.env"),
        format!(
            "CB_HOSTS={hosts}\nCB_SETUP_TOKEN={token}\nCB_SECRET={secret}\nPROJECT_MARK={mark}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("cb_hooks/mark.cb.js"),
        r#"routerAdd("GET", "/mark", (e) => { e.json(200, { v: $os.getenv("PROJECT_MARK") }); })"#,
    )
    .unwrap();
}

async fn send(
    router: &Router,
    host: &str,
    method: Method,
    uri: &str,
    token: Option<&str>,
    extra: Option<(&str, &str)>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", t);
    }
    if let Some((k, v)) = extra {
        req = req.header(k, v);
    }
    let body = body.map_or_else(Body::empty, |v| Body::from(v.to_string()));
    let mut req = req.body(body).unwrap();
    // The real listener supplies this via `into_make_service_with_connect_info`.
    req.extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40000,
        ))));
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

async fn two_projects() -> (tempfile::TempDir, Router, Vec<cratebase_server::app::App>) {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "a", "a.test", "tok-a", SECRET_A, "alpha");
    write_project(root.path(), "b", "b.test", "tok-b", SECRET_B, "beta");
    let (router, apps) = host::build(root.path()).await.expect("build host");
    (root, router, apps)
}

#[tokio::test]
async fn routes_by_host_and_rejects_unknown() {
    let (_root, router, _apps) = two_projects().await;
    let (s, _) = send(
        &router,
        "a.test",
        Method::GET,
        "/api/health",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        &router,
        "B.TEST:8099",
        Method::GET,
        "/api/health",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = send(
        &router,
        "x.test",
        Method::GET,
        "/api/health",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(body["message"], "Unknown project host.");
}

#[tokio::test]
async fn projects_have_separate_databases() {
    let (_root, router, _apps) = two_projects().await;
    let (s, _) = send(
        &router,
        "a.test",
        Method::POST,
        "/api/setup",
        None,
        Some(("x-setup-token", "tok-a")),
        Some(json!({"email":"owner@a.test","password":"password-12345","passwordConfirm":"password-12345"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, a) = send(
        &router,
        "a.test",
        Method::GET,
        "/api/setup/status",
        None,
        None,
        None,
    )
    .await;
    let (_, b) = send(
        &router,
        "b.test",
        Method::GET,
        "/api/setup/status",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(a["needsSetup"], false);
    assert_eq!(b["needsSetup"], true);

    let (s, auth) = send(
        &router,
        "a.test",
        Method::POST,
        "/api/collections/_superusers/auth-with-password",
        None,
        None,
        Some(json!({"identity":"owner@a.test","password":"password-12345"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{auth}");
    let token = auth["token"].as_str().unwrap().to_string();

    let (s, body) = send(
        &router,
        "a.test",
        Method::POST,
        "/api/collections",
        Some(&token),
        None,
        Some(json!({"name":"posts","type":"base","fields":[{"name":"t","type":"text"}]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let (s, _) = send(
        &router,
        "a.test",
        Method::GET,
        "/api/collections/posts",
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // Project b has no such collection (and the token is not valid there).
    let (s, _) = send(
        &router,
        "b.test",
        Method::GET,
        "/api/collections/posts",
        Some(&token),
        None,
        None,
    )
    .await;
    assert!(
        s == StatusCode::NOT_FOUND || s == StatusCode::UNAUTHORIZED,
        "{s}"
    );
    let (s, _) = send(
        &router,
        "b.test",
        Method::GET,
        "/api/collections/posts/records",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn os_getenv_reads_the_project_variables() {
    let (_root, router, _apps) = two_projects().await;
    let (s, a) = send(&router, "a.test", Method::GET, "/mark", None, None, None).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(a["v"], "alpha");
    let (_, b) = send(&router, "b.test", Method::GET, "/mark", None, None, None).await;
    assert_eq!(b["v"], "beta");
}

#[tokio::test]
async fn load_specs_rejects_duplicate_hosts_and_process_wide_keys() {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "a", "same.test", "t", SECRET_A, "x");
    write_project(
        root.path(),
        "b",
        "other.test, SAME.test",
        "t",
        SECRET_B,
        "y",
    );
    let err = host::load_specs(root.path()).unwrap_err().to_string();
    assert!(
        err.contains("same.test") && err.contains('a') && err.contains('b'),
        "{err}"
    );

    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "a", "a.test", "t", SECRET_A, "x");
    let env = root.path().join("a/project.env");
    let mut text = std::fs::read_to_string(&env).unwrap();
    text.push_str("CB_ENCRYPTION=abc\n");
    std::fs::write(&env, text).unwrap();
    let err = host::load_specs(root.path()).unwrap_err().to_string();
    assert_eq!(
        err,
        "CB_ENCRYPTION is process-wide in host mode; set it in the host process environment, not in project.env"
    );

    let empty = tempfile::tempdir().unwrap();
    let err = host::load_specs(empty.path()).unwrap_err().to_string();
    assert!(err.starts_with("no projects found under "), "{err}");
}

#[test]
fn new_project_writes_env_and_refuses_existing() {
    let root = tempfile::tempdir().unwrap();
    let token = host::new_project(root.path(), "nyari", "nyari.test").unwrap();
    assert_eq!(token.len(), 24);
    let specs = host::load_specs(root.path()).unwrap();
    assert_eq!(specs[0].hosts, vec!["nyari.test"]);
    assert!(host::new_project(root.path(), "nyari", "x.test").is_err());
    assert!(host::new_project(root.path(), "Bad Name", "x.test").is_err());
}
