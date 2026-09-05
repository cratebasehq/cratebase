//! Provider-agnostic OAuth2 authorization-code + PKCE plumbing (RFC 6749
//! §4.1, RFC 7636), plus Google's and GitHub's specific userinfo response
//! shapes.
//!
//! Nothing here makes an HTTP request — building the token-exchange body
//! ([`TokenExchange::form`]) and turning a provider's raw JSON response
//! into cratebase's normalized [`OAuth2User`] is pure and unit-testable
//! without a mock server. The actual `reqwest` round trips (token
//! exchange, userinfo fetch) live in `cratebase_server::routes::auth`,
//! next to the `reqwest::Client` `webhooks.rs` already owns — this crate
//! otherwise has no HTTP client dependency at all (see the crate doc).

use serde::Deserialize;
use serde_json::Value;

use crate::error::{AuthError, AuthResult};

/// A provider cratebase knows the endpoints, scopes and userinfo shape
/// for out of the box. Anything else configured through a collection's
/// `OAuth2Provider.authURL`/`tokenURL`/`userInfoURL` is a hand-configured
/// custom provider and falls back to [`parse_generic_userinfo`]'s flat
/// `id`/`name`/`username`/`email`/`avatar` mapping instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownProvider {
    Google,
    GitHub,
}

impl KnownProvider {
    /// Matches a collection's `OAuth2Provider.name` — PocketBase's own
    /// preset names, lowercase.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "google" => Some(Self::Google),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }

    pub fn auth_url(self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::GitHub => "https://github.com/login/oauth/authorize",
        }
    }

    pub fn token_url(self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::GitHub => "https://github.com/login/oauth/access_token",
        }
    }

    pub fn user_info_url(self) -> &'static str {
        match self {
            Self::Google => "https://www.googleapis.com/oauth2/v3/userinfo",
            Self::GitHub => "https://api.github.com/user",
        }
    }

    /// A second endpoint to hit only when the primary userinfo call
    /// didn't already carry a usable email. GitHub's `/user` omits
    /// `email` unless the account's primary address is public, so
    /// `/user/emails` (needs the `user:email` scope — see
    /// [`Self::default_scope`]) is the only reliable way to get one.
    /// Google's userinfo endpoint always returns `email` given the
    /// `.../userinfo.email` scope, so it has no equivalent.
    pub fn emails_url(self) -> Option<&'static str> {
        match self {
            Self::GitHub => Some("https://api.github.com/user/emails"),
            Self::Google => None,
        }
    }

    /// Requested when a provider is enabled without an explicit `scope`
    /// override, matching PocketBase's own built-in provider defaults.
    pub fn default_scope(self) -> &'static str {
        match self {
            Self::Google => {
                "https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/userinfo.email"
            }
            Self::GitHub => "read:user user:email",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::GitHub => "GitHub",
        }
    }
}

/// Everything needed to build a token-exchange POST body (RFC 6749
/// §4.1.3), independent of which HTTP client sends it.
#[derive(Debug, Clone)]
pub struct TokenExchange<'a> {
    pub code: &'a str,
    pub client_id: &'a str,
    pub client_secret: &'a str,
    /// Must equal the `redirect_uri` the authorize-URL request used —
    /// the SDK's own redirect page, not anything cratebase serves.
    pub redirect_uri: &'a str,
    /// The PKCE `code_verifier` matching the `code_challenge` sent to
    /// the authorize URL. `None` only for a provider/collection that
    /// opted out of PKCE.
    pub code_verifier: Option<&'a str>,
}

impl<'a> TokenExchange<'a> {
    /// The `application/x-www-form-urlencoded` pairs for the token
    /// request.
    pub fn form(&self) -> Vec<(&'static str, String)> {
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", self.code.to_string()),
            ("client_id", self.client_id.to_string()),
            ("client_secret", self.client_secret.to_string()),
            ("redirect_uri", self.redirect_uri.to_string()),
        ];
        if let Some(verifier) = self.code_verifier {
            form.push(("code_verifier", verifier.to_string()));
        }
        form
    }
}

/// The token endpoint's JSON response. Both providers here answer JSON
/// when asked for it with an `Accept: application/json` header (GitHub's
/// default is otherwise form-encoded).
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

pub fn parse_token_response(body: &[u8]) -> AuthResult<TokenResponse> {
    serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("token response: {e}")))
}

/// cratebase's normalized shape for whatever a provider's userinfo
/// endpoint(s) returned — what an `_externalAuths` link and a new
/// record's mapped fields are built from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OAuth2User {
    /// The provider's own stable subject id (`sub` for Google, the
    /// numeric `id` for GitHub) — this, not the email, is what
    /// `_externalAuths` links against, since an email can change or be
    /// reused by a different account.
    pub id: String,
    pub name: String,
    pub username: String,
    pub email: String,
    pub avatar_url: String,
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    sub: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    /// CRITICAL 4: an attacker with an unverified Google/Workspace
    /// address must not be able to sign into (and silently verify) an
    /// existing record sharing that email — `email` below is only kept
    /// when this is genuinely `true`.
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    picture: String,
}

/// Parses `GET https://www.googleapis.com/oauth2/v3/userinfo`'s response.
pub fn parse_google_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let info: GoogleUserInfo = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("google userinfo: {e}")))?;
    Ok(OAuth2User {
        id: info.sub,
        name: info.name,
        username: String::new(),
        email: if info.email_verified {
            info.email
        } else {
            String::new()
        },
        avatar_url: info.picture,
    })
}

#[derive(Deserialize)]
struct GitHubUser {
    id: i64,
    #[serde(default)]
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    avatar_url: String,
}

#[derive(Deserialize)]
struct GitHubEmail {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

/// Parses `GET https://api.github.com/user`, falling back to
/// `emails_body` (`GET .../user/emails`, see [`KnownProvider::emails_url`])
/// for the email only when the primary call's `email` was empty. Prefers
/// the `primary && verified` address, then any `verified` one — an
/// unverified address is never attributed to this login, since GitHub
/// lets an account attach one before proving ownership.
pub fn parse_github_userinfo(
    user_body: &[u8],
    emails_body: Option<&[u8]>,
) -> AuthResult<OAuth2User> {
    let user: GitHubUser = serde_json::from_slice(user_body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("github userinfo: {e}")))?;
    let mut email = user.email.unwrap_or_default();
    if email.is_empty() {
        if let Some(body) = emails_body {
            if let Ok(emails) = serde_json::from_slice::<Vec<GitHubEmail>>(body) {
                email = emails
                    .iter()
                    .find(|e| e.primary && e.verified)
                    .or_else(|| emails.iter().find(|e| e.verified))
                    .map(|e| e.email.clone())
                    .unwrap_or_default();
            }
        }
    }
    Ok(OAuth2User {
        id: user.id.to_string(),
        name: user.name.unwrap_or_default(),
        username: user.login,
        email,
        avatar_url: user.avatar_url,
    })
}

/// The flat shape a hand-configured (non-[`KnownProvider`]) OAuth2/OpenID
/// userinfo endpoint is expected to answer with — PocketBase's own
/// fallback field names for a custom provider.
pub fn parse_generic_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let raw: Value = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("userinfo: {e}")))?;
    let get = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| raw.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    // CRITICAL 4: same reasoning as `parse_google_userinfo` — a
    // hand-configured provider's `email` is only trusted when its own
    // `email_verified` claim (any of Go `cast.ToBool`'s truthy shapes:
    // `true`, a non-zero number, or `"true"`/`"1"`) is actually present
    // and true, so an attacker with an unverified address on that
    // provider cannot sign into and verify an existing record sharing
    // it.
    let email_verified = raw.get("email_verified").is_some_and(is_truthy);
    Ok(OAuth2User {
        id: get(&["id", "sub"]),
        name: get(&["name"]),
        username: get(&["username", "login", "preferred_username"]),
        email: if email_verified {
            get(&["email"])
        } else {
            String::new()
        },
        avatar_url: get(&["avatar", "avatar_url", "picture"]),
    })
}

/// Go's `cast.ToBool`, which is what a truthy claim means throughout
/// PocketBase-compatible rule/field handling (see
/// `cratebase_server::routes::records`'s own `truthy` for the record
/// field version of the same rule).
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => matches!(s.as_str(), "true" | "1" | "t" | "TRUE" | "True"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_providers_resolve_by_name_and_only_by_name() {
        assert_eq!(
            KnownProvider::from_name("google"),
            Some(KnownProvider::Google)
        );
        assert_eq!(
            KnownProvider::from_name("github"),
            Some(KnownProvider::GitHub)
        );
        assert_eq!(KnownProvider::from_name("Google"), None);
        assert_eq!(KnownProvider::from_name("gitlab"), None);
    }

    #[test]
    fn token_exchange_form_omits_code_verifier_when_absent() {
        let exchange = TokenExchange {
            code: "auth-code",
            client_id: "cid",
            client_secret: "csecret",
            redirect_uri: "https://app.example/redirect",
            code_verifier: None,
        };
        let form = exchange.form();
        assert!(!form.iter().any(|(k, _)| *k == "code_verifier"));
        assert!(form.contains(&("grant_type", "authorization_code".to_string())));
        assert!(form.contains(&("code", "auth-code".to_string())));
        assert!(form.contains(&("client_id", "cid".to_string())));
        assert!(form.contains(&("client_secret", "csecret".to_string())));
        assert!(form.contains(&("redirect_uri", "https://app.example/redirect".to_string())));
    }

    #[test]
    fn token_exchange_form_carries_pkce_verifier_when_present() {
        let exchange = TokenExchange {
            code: "auth-code",
            client_id: "cid",
            client_secret: "csecret",
            redirect_uri: "https://app.example/redirect",
            code_verifier: Some("the-verifier"),
        };
        assert!(exchange
            .form()
            .contains(&("code_verifier", "the-verifier".to_string())));
    }

    #[test]
    fn parses_token_response_ignoring_unknown_extra_fields() {
        let body =
            br#"{"access_token":"tok","token_type":"bearer","expires_in":3599,"scope":"email"}"#;
        let parsed = parse_token_response(body).unwrap();
        assert_eq!(parsed.access_token, "tok");
        assert_eq!(parsed.token_type, "bearer");
        assert_eq!(parsed.scope.as_deref(), Some("email"));
    }

    #[test]
    fn rejects_a_token_response_with_no_access_token() {
        assert!(parse_token_response(br#"{"error":"invalid_grant"}"#).is_err());
    }

    #[test]
    fn parses_google_userinfo() {
        let body = br#"{
            "sub": "10769150350006150715113082367",
            "name": "Jo March",
            "given_name": "Jo",
            "email": "jo@example.com",
            "email_verified": true,
            "picture": "https://example.com/jo.jpg"
        }"#;
        let user = parse_google_userinfo(body).unwrap();
        assert_eq!(
            user,
            OAuth2User {
                id: "10769150350006150715113082367".into(),
                name: "Jo March".into(),
                username: String::new(),
                email: "jo@example.com".into(),
                avatar_url: "https://example.com/jo.jpg".into(),
            }
        );
    }

    #[test]
    fn google_never_attributes_an_unverified_email() {
        let body = br#"{
            "sub": "10769150350006150715113082367",
            "name": "Jo March",
            "email": "jo@example.com",
            "email_verified": false,
            "picture": "https://example.com/jo.jpg"
        }"#;
        let user = parse_google_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn google_never_attributes_an_email_with_no_verified_claim_at_all() {
        let body = br#"{"sub": "1", "email": "jo@example.com"}"#;
        let user = parse_google_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_github_userinfo_with_a_public_email() {
        let body = br#"{
            "id": 583231,
            "login": "octocat",
            "name": "The Octocat",
            "email": "octocat@github.com",
            "avatar_url": "https://avatars.githubusercontent.com/u/583231"
        }"#;
        let user = parse_github_userinfo(body, None).unwrap();
        assert_eq!(
            user,
            OAuth2User {
                id: "583231".into(),
                name: "The Octocat".into(),
                username: "octocat".into(),
                email: "octocat@github.com".into(),
                avatar_url: "https://avatars.githubusercontent.com/u/583231".into(),
            }
        );
    }

    #[test]
    fn github_falls_back_to_the_primary_verified_email() {
        let user_body = br#"{"id": 1, "login": "jo", "email": null, "avatar_url": ""}"#;
        let emails_body = br#"[
            {"email": "old@example.com", "primary": false, "verified": true},
            {"email": "jo@example.com", "primary": true, "verified": true},
            {"email": "unverified@example.com", "primary": false, "verified": false}
        ]"#;
        let user = parse_github_userinfo(user_body, Some(emails_body)).unwrap();
        assert_eq!(user.email, "jo@example.com");
    }

    #[test]
    fn github_never_attributes_an_unverified_email() {
        let user_body = br#"{"id": 1, "login": "jo", "email": null, "avatar_url": ""}"#;
        let emails_body =
            br#"[{"email": "spoofed@example.com", "primary": true, "verified": false}]"#;
        let user = parse_github_userinfo(user_body, Some(emails_body)).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_generic_userinfo_with_alternate_key_names() {
        let body = br#"{"sub": "abc123", "name": "Jo", "login": "jomarch", "email": "jo@example.com", "picture": "https://example.com/jo.jpg"}"#;
        let user = parse_generic_userinfo(body).unwrap();
        assert_eq!(user.id, "abc123");
        assert_eq!(user.username, "jomarch");
        assert_eq!(user.avatar_url, "https://example.com/jo.jpg");
        assert_eq!(user.email, "", "no email_verified claim: email dropped");
    }

    #[test]
    fn generic_provider_keeps_a_truthily_verified_email() {
        let body = br#"{"sub": "abc123", "email": "jo@example.com", "email_verified": "true"}"#;
        let user = parse_generic_userinfo(body).unwrap();
        assert_eq!(user.email, "jo@example.com");
    }

    #[test]
    fn generic_provider_never_attributes_an_unverified_email() {
        let body = br#"{"sub": "abc123", "email": "spoofed@example.com", "email_verified": false}"#;
        let user = parse_generic_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }
}
