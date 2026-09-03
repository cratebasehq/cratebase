//! Generic OAuth2 authorization-code flow support. Cratebase never redirects
//! the browser itself (it has no frontend of its own to redirect back to)
//! — the client builds/opens the provider's `authUrl` (from `auth-methods`)
//! with its own redirect URI, receives `code` there, and hands it to
//! `POST /collections/{c}/auth-with-oauth2` for the server-side exchange.
//!
//! Each provider needs three endpoints (authorize/token/userinfo) and a
//! provider-specific response shape for the userinfo call — that's the
//! only part `fetch_user` branches on. Adding a new provider means adding
//! a branch to `providers_from_env` and `fetch_user`, not a new trait: the
//! shapes are different enough (GitHub's email is a separate scoped call)
//! that a generic trait would just be a thin wrapper around a `match`
//! anyway.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: &'static str,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub userinfo_url: &'static str,
    pub scope: &'static str,
}

pub struct ExternalUser {
    pub provider_user_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
    #[error("http request to provider failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider rejected the request: {0}")]
    Provider(String),
    #[error("provider response was missing an access token")]
    MissingAccessToken,
    #[error("provider account has no email address")]
    NoEmail,
}

/// Reads `OAUTH_<PROVIDER>_CLIENT_ID`/`OAUTH_<PROVIDER>_CLIENT_SECRET` —
/// a provider is only offered if both are set, so an operator who hasn't
/// configured any OAuth2 app just gets an empty list, not an error.
pub fn providers_from_env() -> Vec<ProviderConfig> {
    let mut providers = Vec::new();
    if let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("OAUTH_GOOGLE_CLIENT_ID"),
        std::env::var("OAUTH_GOOGLE_CLIENT_SECRET"),
    ) {
        providers.push(ProviderConfig {
            name: "google",
            client_id,
            client_secret,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: "https://oauth2.googleapis.com/token",
            userinfo_url: "https://www.googleapis.com/oauth2/v3/userinfo",
            scope: "openid email profile",
        });
    }
    if let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("OAUTH_GITHUB_CLIENT_ID"),
        std::env::var("OAUTH_GITHUB_CLIENT_SECRET"),
    ) {
        providers.push(ProviderConfig {
            name: "github",
            client_id,
            client_secret,
            auth_url: "https://github.com/login/oauth/authorize",
            token_url: "https://github.com/login/oauth/access_token",
            userinfo_url: "https://api.github.com/user",
            scope: "read:user user:email",
        });
    }
    providers
}

/// Exchanges an authorization `code` for an access token. `redirect_uri`
/// must exactly match the one the client used to obtain `code` — the
/// providers we support both enforce this as part of the OAuth2 spec.
pub async fn exchange_code(
    provider: &ProviderConfig,
    code: &str,
    redirect_uri: &str,
) -> Result<String, OAuth2Error> {
    let client = reqwest::Client::new();
    let form = [
        ("client_id", provider.client_id.as_str()),
        ("client_secret", provider.client_secret.as_str()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];
    let res = client
        .post(provider.token_url)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(OAuth2Error::Provider(res.text().await.unwrap_or_default()));
    }
    let body: Value = res.json().await?;
    body.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(OAuth2Error::MissingAccessToken)
}

/// Fetches the provider's userinfo endpoint and normalizes the
/// provider-specific response shape.
pub async fn fetch_user(
    provider: &ProviderConfig,
    access_token: &str,
) -> Result<ExternalUser, OAuth2Error> {
    let client = reqwest::Client::new();
    let body: Value = client
        .get(provider.userinfo_url)
        .bearer_auth(access_token)
        .header("user-agent", "cratebase")
        .send()
        .await?
        .json()
        .await?;

    match provider.name {
        "google" => Ok(ExternalUser {
            provider_user_id: body["sub"].as_str().unwrap_or_default().to_string(),
            email: body["email"].as_str().map(str::to_string),
            name: body["name"].as_str().map(str::to_string),
        }),
        "github" => {
            let provider_user_id = body["id"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_default();
            let mut email = body["email"].as_str().map(str::to_string);
            if email.is_none() {
                // A private-by-default primary email doesn't show up on
                // `/user` at all — it's only reachable via this
                // separately scoped endpoint.
                if let Ok(res) = client
                    .get("https://api.github.com/user/emails")
                    .bearer_auth(access_token)
                    .header("user-agent", "cratebase")
                    .send()
                    .await
                {
                    if let Ok(emails) = res.json::<Vec<Value>>().await {
                        email = emails
                            .iter()
                            .find(|e| e["primary"].as_bool().unwrap_or(false))
                            .and_then(|e| e["email"].as_str())
                            .map(str::to_string);
                    }
                }
            }
            Ok(ExternalUser {
                provider_user_id,
                email,
                name: body["name"].as_str().map(str::to_string),
            })
        }
        _ => Err(OAuth2Error::Provider(format!(
            "no userinfo parser for provider '{}'",
            provider.name
        ))),
    }
}
