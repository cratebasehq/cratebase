//! `POST /api/api-keys` — mint a new `_api_keys` row and hand back the
//! raw key exactly once.
//!
//! Every other operation on `_api_keys` (list/view/update/delete) goes
//! through the ordinary generic Records API at
//! `/api/collections/_api_keys/records`, already superuser-gated end to
//! end by the collection's own rules (see
//! `cratebase_core::Collection::default_system_collections`'s doc
//! comment on `_api_keys`). This route exists only because that API can
//! never round-trip `key` — it's `hidden`, and the stored value is a
//! one-way hash anyway, so there is nothing to return from a normal
//! create. Minting therefore needs its own endpoint that generates the
//! raw key, hashes it once, and returns the only copy the caller will
//! ever see; see `crate::api_keys` for the generation and hashing.
//!
//! `actsAsCollection`/`actsAsRecord` (see `crate::api_keys`'s module doc's
//! "Scoping" section) are optional and validated here, at mint time,
//! through `crate::api_keys::load_acts_as` — the same lookup `resolve`
//! runs on every request — so a typo'd collection name or a record id
//! that doesn't exist is rejected before a key that can never
//! authenticate gets minted, rather than failing silently later.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use cratebase_core::Record;
use cratebase_db::records;

use crate::app::App;
use crate::extract::RequireSuperuser;
use crate::http_error::{ApiError, ApiJson, ApiResult};

pub fn router() -> Router<App> {
    Router::new().route("/api-keys", post(create))
}

#[derive(Debug, Default, Deserialize)]
struct CreateBody {
    #[serde(default)]
    name: String,
    /// Both or neither — see the module doc.
    #[serde(default, rename = "actsAsCollection")]
    acts_as_collection: Option<String>,
    #[serde(default, rename = "actsAsRecord")]
    acts_as_record: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    id: String,
    name: String,
    prefix: String,
    acts_as_collection: String,
    acts_as_record: String,
    /// The only time this ever leaves the server — see the module doc.
    key: String,
}

async fn create(
    State(app): State<App>,
    _su: RequireSuperuser,
    ApiJson(body): ApiJson<CreateBody>,
) -> ApiResult<Json<CreateResponse>> {
    let collection = app
        .db()
        .collections
        .get_by_name(crate::api_keys::COLLECTION)
        .ok_or_else(|| ApiError::internal(format!("{} is missing", crate::api_keys::COLLECTION)))?;

    let acts_as = match (&body.acts_as_collection, &body.acts_as_record) {
        (None, None) => None,
        (Some(c), Some(r)) if !c.is_empty() && !r.is_empty() => {
            if crate::api_keys::load_acts_as(&app, c, r).await.is_none() {
                return Err(ApiError::bad_request(format!(
                    "no record {r:?} in an auth collection named {c:?}"
                )));
            }
            Some((c.clone(), r.clone()))
        }
        (Some(c), Some(r)) if c.is_empty() && r.is_empty() => None,
        _ => {
            return Err(ApiError::bad_request(
                "actsAsCollection and actsAsRecord must both be set, or both omitted",
            ));
        }
    };

    let generated = crate::api_keys::generate();
    let hash = cratebase_auth::hash_password_async(&generated.raw)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let (acts_as_collection, acts_as_record) = acts_as.unwrap_or_default();

    let mut record = Record::new(collection.clone());
    record.set("name", Value::String(body.name.clone()));
    record.set("key", Value::String(hash));
    record.set("prefix", Value::String(generated.prefix.clone()));
    record.set("enabled", Value::Bool(true));
    record.set(
        "actsAsCollection",
        Value::String(acts_as_collection.clone()),
    );
    record.set("actsAsRecord", Value::String(acts_as_record.clone()));
    records::create(app.db(), &app.db().collections, &mut record).await?;

    Ok(Json(CreateResponse {
        id: record.id().to_string(),
        name: body.name,
        prefix: generated.prefix,
        acts_as_collection,
        acts_as_record,
        key: generated.raw,
    }))
}

#[cfg(test)]
mod scoping_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cratebase_core::Record;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::app::App;
    use crate::config::Config;

    async fn test_app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::new(Config::memory(dir.path()));
        app.bootstrap().await.expect("bootstrap");
        (app, dir)
    }

    async fn superuser_token(app: &App) -> String {
        let id = app
            .create_superuser("admin@example.com", "password12345")
            .await
            .expect("create superuser");
        app.mint_token("_superusers", &id, cratebase_auth::TokenType::Auth, 3600)
            .await
            .expect("mint token")
    }

    /// A plain `users` record — no password flow, no session — the same
    /// shortcut `routes::auth`'s and `crate::audit`'s own tests take to
    /// seed an ordinary auth-collection record directly.
    async fn seed_user(app: &App, email: &str) -> String {
        let users = app.db().collections.get_by_name("users").expect("users");
        let mut record = Record::new(users);
        record.set("email", Value::String(email.into()));
        record.set("password", Value::String("whatever-password".into()));
        record.set("tokenKey", Value::String(crate::app::new_token_key()));
        cratebase_db::records::create(app.db(), &app.db().collections, &mut record)
            .await
            .expect("seed user");
        record.id().to_string()
    }

    fn json_request(method: &str, uri: &str, token: &str, body: Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn empty_request(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_rejects_a_nonexistent_target_collection() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let user_id = seed_user(&app, "a@example.com").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let response = router
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "k", "actsAsCollection": "no_such_collection", "actsAsRecord": user_id }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_rejects_a_nonexistent_target_record() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let response = router
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "k", "actsAsCollection": "users", "actsAsRecord": "nonexistent00000000000" }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_rejects_a_non_auth_target_collection() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        // `_api_keys` itself is a base collection, not auth — there is no
        // login-shaped `Auth` a row in it could ever produce.
        let response = router
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "k", "actsAsCollection": "_api_keys", "actsAsRecord": "whatever00000000000000" }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_rejects_only_one_of_the_pair() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let response = router
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "k", "actsAsCollection": "users" }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The load-bearing test: a key scoped to userA's own record gets
    /// exactly userA's rule-gated access — it can read its own record
    /// (`users`' default `view_rule` is `id = @request.auth.id`) but not
    /// userB's, precisely as userA logging in normally could not.
    #[tokio::test]
    async fn scoped_key_is_subject_to_the_target_records_own_rules() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let user_a = seed_user(&app, "a@example.com").await;
        let user_b = seed_user(&app, "b@example.com").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let create_response = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "agent", "actsAsCollection": "users", "actsAsRecord": user_a }),
            ))
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::OK);
        let minted = json_body(create_response).await;
        assert_eq!(minted["actsAsCollection"], json!("users"));
        assert_eq!(minted["actsAsRecord"], json!(user_a));
        let key_token = minted["key"].as_str().unwrap();

        let own = router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/collections/users/records/{user_a}"),
                key_token,
            ))
            .await
            .unwrap();
        assert_eq!(own.status(), StatusCode::OK);

        let other = router
            .oneshot(empty_request(
                "GET",
                &format!("/collections/users/records/{user_b}"),
                key_token,
            ))
            .await
            .unwrap();
        assert_eq!(
            other.status(),
            StatusCode::NOT_FOUND,
            "a rule-denied view is masked as not-found, same as it would be for userA logged in normally"
        );
    }

    /// Regression: a key minted with no `actsAs*` at all keeps behaving
    /// exactly as before this feature — unscoped superuser, unrestricted
    /// by any collection's rules.
    #[tokio::test]
    async fn an_unscoped_key_is_still_an_unrestricted_superuser() {
        let (app, _dir) = test_app().await;
        let admin_token = superuser_token(&app).await;
        let user_a = seed_user(&app, "a@example.com").await;
        let router = crate::routes::api_router(&app).with_state(app.clone());

        let create_response = router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api-keys",
                &admin_token,
                json!({ "name": "ci" }),
            ))
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::OK);
        let minted = json_body(create_response).await;
        assert_eq!(minted["actsAsCollection"], json!(""));
        assert_eq!(minted["actsAsRecord"], json!(""));
        let key_token = minted["key"].as_str().unwrap();

        let response = router
            .oneshot(empty_request(
                "GET",
                &format!("/collections/users/records/{user_a}"),
                key_token,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "an unscoped key must remain unrestricted root, exactly like before this feature"
        );
    }
}
