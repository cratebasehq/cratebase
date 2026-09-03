use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use cratebase_auth::{verify_token, TokenKind};
use cratebase_core::AppError;
use cratebase_db::{admins, records, AuthContext};
use serde_json::Map;

use crate::http_error::ApiError;
use crate::state::AppState;

fn bearer_token(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// The caller's identity, resolved from a `Bearer` token if present and
/// valid. An absent, malformed, or expired token degrades to "anonymous"
/// rather than a hard 401 — individual handlers decide whether auth is
/// required (most collection rules are perfectly happy with a public
/// caller).
#[derive(Clone, Default)]
pub struct CurrentAuth(pub Option<AuthContext>);

impl<S> FromRequestParts<S> for CurrentAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let Some(token) = bearer_token(parts) else {
            return Ok(CurrentAuth(None));
        };
        let Ok(claims) = verify_token(token, &app.config.auth_secret) else {
            return Ok(CurrentAuth(None));
        };
        if !claims.kind.is_session_kind() {
            // A verify-email/reset-password/change-email link token is
            // single-purpose; it must never authenticate an ordinary
            // request, only its own `confirm-*` endpoint (which decodes
            // it directly, bypassing this extractor).
            return Ok(CurrentAuth(None));
        }

        match claims.kind {
            TokenKind::Admin => {
                if admins::get_admin_by_id(&app.db, &claims.sub).await.is_ok() {
                    Ok(CurrentAuth(Some(AuthContext {
                        id: claims.sub,
                        collection_id: String::new(),
                        is_superuser: true,
                        record: Map::new(),
                    })))
                } else {
                    Ok(CurrentAuth(None))
                }
            }
            TokenKind::Auth => {
                let Ok(collection) =
                    cratebase_db::collections::get_collection_by_id(&app.db, &claims.collection_id)
                        .await
                else {
                    return Ok(CurrentAuth(None));
                };
                match records::get_record(&app.db, &collection, &claims.sub, None).await {
                    Ok(record) => {
                        let record_map = record.as_object().cloned().unwrap_or_default();
                        Ok(CurrentAuth(Some(AuthContext {
                            id: claims.sub,
                            collection_id: claims.collection_id,
                            is_superuser: false,
                            record: record_map,
                        })))
                    }
                    Err(_) => Ok(CurrentAuth(None)),
                }
            }
            // Filtered out above by `is_session_kind()`.
            TokenKind::VerifyEmail
            | TokenKind::ResetPassword
            | TokenKind::ChangeEmail
            | TokenKind::Mfa
            | TokenKind::FileToken => Ok(CurrentAuth(None)),
        }
    }
}

/// Like [`CurrentAuth`] but rejects the request with 401 unless the caller
/// is an authenticated superuser. Used for schema/collection management
/// endpoints.
pub struct RequireAdmin(pub AuthContext);

impl<S> FromRequestParts<S> for RequireAdmin
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let CurrentAuth(auth) = CurrentAuth::from_request_parts(parts, state).await.unwrap();
        match auth {
            Some(ctx) if ctx.is_superuser => Ok(RequireAdmin(ctx)),
            _ => Err(ApiError(AppError::Unauthorized(
                "a superuser token is required".into(),
            ))),
        }
    }
}
