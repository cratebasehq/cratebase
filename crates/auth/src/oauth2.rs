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
use serde_json::{Map, Value};

use crate::error::{AuthError, AuthResult};

/// A provider cratebase knows the endpoints, scopes and userinfo shape
/// for out of the box. Anything else configured through a collection's
/// `OAuth2Provider.authURL`/`tokenURL`/`userInfoURL` is a hand-configured
/// custom provider and falls back to [`parse_generic_userinfo`]'s flat
/// `id`/`name`/`username`/`email`/`avatar` mapping instead — including a
/// generic OIDC provider (`extra.issuer` set, discovered at runtime; see
/// `cratebase_server::routes::oidc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownProvider {
    Google,
    GitHub,
    Apple,
    Microsoft,
    Discord,
    GitLab,
    Facebook,
    /// X (formerly Twitter). OAuth2 with PKCE is mandatory — there is no
    /// non-PKCE fallback, unlike the other presets.
    Twitter,
    LinkedIn,
    Slack,
    Twitch,
    Spotify,
}

impl KnownProvider {
    /// Matches a collection's `OAuth2Provider.name` — PocketBase's own
    /// preset names, lowercase.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "google" => Some(Self::Google),
            "github" => Some(Self::GitHub),
            "apple" => Some(Self::Apple),
            "microsoft" => Some(Self::Microsoft),
            "discord" => Some(Self::Discord),
            "gitlab" => Some(Self::GitLab),
            "facebook" => Some(Self::Facebook),
            "twitter" => Some(Self::Twitter),
            "linkedin" => Some(Self::LinkedIn),
            "slack" => Some(Self::Slack),
            "twitch" => Some(Self::Twitch),
            "spotify" => Some(Self::Spotify),
            _ => None,
        }
    }

    /// The Entra ID tenant segment for [`Self::Microsoft`]'s URLs —
    /// `extra.tenant`, defaulting to `"common"` (personal + work/school
    /// accounts), matching PocketBase's own Microsoft preset default.
    fn microsoft_tenant(extra: &Map<String, Value>) -> String {
        extra
            .get("tenant")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("common")
            .to_string()
    }

    /// The GitLab instance base URL for [`Self::GitLab`] — `extra.baseUrl`,
    /// defaulting to `https://gitlab.com` for self-hosted instances.
    fn gitlab_base(extra: &Map<String, Value>) -> String {
        extra
            .get("baseUrl")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("https://gitlab.com")
            .trim_end_matches('/')
            .to_string()
    }

    /// The authorization endpoint. Takes the provider's `extra` config
    /// because [`Self::Microsoft`] (tenant) and [`Self::GitLab`]
    /// (self-hosted base URL) can't be a fixed constant.
    pub fn auth_url(self, extra: &Map<String, Value>) -> String {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            Self::GitHub => "https://github.com/login/oauth/authorize".to_string(),
            Self::Apple => "https://appleid.apple.com/auth/authorize".to_string(),
            Self::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
                Self::microsoft_tenant(extra)
            ),
            Self::Discord => "https://discord.com/oauth2/authorize".to_string(),
            Self::GitLab => format!("{}/oauth/authorize", Self::gitlab_base(extra)),
            Self::Facebook => "https://www.facebook.com/v19.0/dialog/oauth".to_string(),
            Self::Twitter => "https://twitter.com/i/oauth2/authorize".to_string(),
            Self::LinkedIn => "https://www.linkedin.com/oauth/v2/authorization".to_string(),
            Self::Slack => "https://slack.com/openid/connect/authorize".to_string(),
            Self::Twitch => "https://id.twitch.tv/oauth2/authorize".to_string(),
            Self::Spotify => "https://accounts.spotify.com/authorize".to_string(),
        }
    }

    pub fn token_url(self, extra: &Map<String, Value>) -> String {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token".to_string(),
            Self::GitHub => "https://github.com/login/oauth/access_token".to_string(),
            Self::Apple => "https://appleid.apple.com/auth/token".to_string(),
            Self::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                Self::microsoft_tenant(extra)
            ),
            Self::Discord => "https://discord.com/api/oauth2/token".to_string(),
            Self::GitLab => format!("{}/oauth/token", Self::gitlab_base(extra)),
            Self::Facebook => "https://graph.facebook.com/v19.0/oauth/access_token".to_string(),
            Self::Twitter => "https://api.twitter.com/2/oauth2/token".to_string(),
            Self::LinkedIn => "https://www.linkedin.com/oauth/v2/accessToken".to_string(),
            Self::Slack => "https://slack.com/api/openid.connect.token".to_string(),
            Self::Twitch => "https://id.twitch.tv/oauth2/token".to_string(),
            Self::Spotify => "https://accounts.spotify.com/api/token".to_string(),
        }
    }

    /// Apple has no userinfo endpoint at all (identity is read from the
    /// token response's `id_token` — see `parse_apple_id_token`), so this
    /// answers an empty string for it; callers must special-case `Apple`
    /// the same way they already special-case its lack of an
    /// [`Self::emails_url`].
    pub fn user_info_url(self, extra: &Map<String, Value>) -> String {
        match self {
            Self::Google => "https://www.googleapis.com/oauth2/v3/userinfo".to_string(),
            Self::GitHub => "https://api.github.com/user".to_string(),
            Self::Apple => String::new(),
            Self::Microsoft => "https://graph.microsoft.com/v1.0/me".to_string(),
            Self::Discord => "https://discord.com/api/users/@me".to_string(),
            Self::GitLab => format!("{}/api/v4/user", Self::gitlab_base(extra)),
            Self::Facebook => {
                "https://graph.facebook.com/me?fields=id,name,email,picture.type(large)"
                    .to_string()
            }
            Self::Twitter => {
                "https://api.twitter.com/2/users/me?user.fields=profile_image_url".to_string()
            }
            Self::LinkedIn => "https://api.linkedin.com/v2/userinfo".to_string(),
            Self::Slack => "https://slack.com/api/openid.connect.userInfo".to_string(),
            Self::Twitch => "https://api.twitch.tv/helix/users".to_string(),
            Self::Spotify => "https://api.spotify.com/v1/me".to_string(),
        }
    }

    /// A second endpoint to hit only when the primary userinfo call
    /// didn't already carry a usable email. GitHub's `/user` omits
    /// `email` unless the account's primary address is public, so
    /// `/user/emails` (needs the `user:email` scope — see
    /// [`Self::default_scope`]) is the only reliable way to get one.
    /// Google's userinfo endpoint always returns `email` given the
    /// `.../userinfo.email` scope, so it has no equivalent — nor does any
    /// other preset here, all of which carry `email`/`email_verified`
    /// directly on their primary userinfo response.
    pub fn emails_url(self) -> Option<&'static str> {
        match self {
            Self::GitHub => Some("https://api.github.com/user/emails"),
            _ => None,
        }
    }

    /// Twitch's Helix API requires the app's `Client-Id` header on every
    /// request, in addition to the bearer token — the only preset here
    /// with that requirement.
    pub fn requires_client_id_header(self) -> bool {
        matches!(self, Self::Twitch)
    }

    /// Requested when a provider is enabled without an explicit `scope`
    /// override, matching PocketBase's own built-in provider defaults
    /// (or, where PocketBase has no preset, that provider's own
    /// minimum-viable "profile + email" scope).
    pub fn default_scope(self) -> &'static str {
        match self {
            Self::Google => {
                "https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/userinfo.email"
            }
            Self::GitHub => "read:user user:email",
            Self::Apple => "name email",
            Self::Microsoft => "openid profile email User.Read",
            Self::Discord => "identify email",
            Self::GitLab => "read_user",
            Self::Facebook => "email public_profile",
            Self::Twitter => "tweet.read users.read offline.access",
            Self::LinkedIn => "openid profile email",
            Self::Slack => "openid profile email",
            Self::Twitch => "user:read:email",
            Self::Spotify => "user-read-email user-read-private",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::GitHub => "GitHub",
            Self::Apple => "Apple",
            Self::Microsoft => "Microsoft",
            Self::Discord => "Discord",
            Self::GitLab => "GitLab",
            Self::Facebook => "Facebook",
            Self::Twitter => "X (Twitter)",
            Self::LinkedIn => "LinkedIn",
            Self::Slack => "Slack",
            Self::Twitch => "Twitch",
            Self::Spotify => "Spotify",
        }
    }

    /// Whether the authorize/token flow supports (and, for
    /// [`Self::Twitter`], requires) PKCE. Every preset here does; kept as
    /// a method so a future preset without PKCE support has somewhere to
    /// say so, and so callers don't have to hardcode "PKCE unless told
    /// otherwise" themselves.
    pub fn supports_pkce(self) -> bool {
        true
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

/// Apple's Sign in with Apple client secret is not a static string: it's
/// a JWT, signed with the team's ES256 private key, that Apple's token
/// endpoint accepts in place of one. Regenerated per token-exchange
/// request rather than cached, since minting one is cheap (a single
/// ES256 signature) and this sidesteps ever persisting a stale one past
/// its `exp`.
///
/// * `iss` — the Apple Developer Team ID (`extra.teamId`)
/// * `sub` — the Service ID (`OAuth2Provider.clientId`)
/// * `aud` — always `https://appleid.apple.com`
/// * header `kid` — the private key's Key ID (`extra.keyId`)
///
/// `private_key_pem` is the `.p8` key's PEM contents (`-----BEGIN
/// PRIVATE KEY-----...`). Apple caps the secret's lifetime at 6 months;
/// this mints one valid for 5 minutes, comfortably past the token
/// exchange it's generated for.
pub fn apple_client_secret(
    team_id: &str,
    key_id: &str,
    private_key_pem: &str,
    client_id: &str,
    now: i64,
) -> AuthResult<String> {
    #[derive(serde::Serialize)]
    struct AppleClaims<'a> {
        iss: &'a str,
        iat: i64,
        exp: i64,
        aud: &'a str,
        sub: &'a str,
    }
    let claims = AppleClaims {
        iss: team_id,
        iat: now,
        exp: now + 300,
        aud: "https://appleid.apple.com",
        sub: client_id,
    };
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(key_id.to_string());
    let key = jsonwebtoken::EncodingKey::from_ec_pem(private_key_pem.as_bytes())
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("apple private key: {e}")))?;
    jsonwebtoken::encode(&header, &claims, &key)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("apple client secret: {e}")))
}

#[derive(Deserialize)]
struct AppleIdTokenClaims {
    sub: String,
    #[serde(default)]
    email: String,
    /// Apple sends this as either a JSON bool or the string `"true"`/
    /// `"false"` depending on API version — accept both.
    #[serde(default)]
    email_verified: Value,
}

/// Parses the already-verified (see `cratebase_server::routes::oidc`'s
/// JWKS-backed verifier, which this crate has no HTTP client to run
/// itself) claims of Apple's token-response `id_token` — the only place
/// Apple exposes the user's identity, since it has no userinfo endpoint.
pub fn parse_apple_id_token_claims(claims_json: &[u8]) -> AuthResult<OAuth2User> {
    let claims: AppleIdTokenClaims = serde_json::from_slice(claims_json)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("apple id_token: {e}")))?;
    let verified = is_truthy(&claims.email_verified);
    Ok(OAuth2User {
        id: claims.sub,
        name: String::new(),
        username: String::new(),
        email: if verified { claims.email } else { String::new() },
        avatar_url: String::new(),
    })
}

/// Apple's `user` form field (only sent, as a JSON string, on the very
/// first authorization of a given Service ID — every later login omits
/// it entirely, so a record's name has exactly one chance to be filled
/// in from it). Shape: `{"name":{"firstName":"...","lastName":"..."},
/// "email":"..."}`. Returns just the display name, since the email
/// already comes from the (verified) `id_token`.
pub fn parse_apple_first_login_name(user_field: &str) -> String {
    let Ok(raw) = serde_json::from_str::<Value>(user_field) else {
        return String::new();
    };
    let name = raw.get("name");
    let first = name
        .and_then(|n| n.get("firstName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let last = name
        .and_then(|n| n.get("lastName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    [first, last].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

#[derive(Deserialize)]
struct MicrosoftUser {
    id: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    mail: Option<String>,
    #[serde(rename = "userPrincipalName", default)]
    user_principal_name: String,
}

/// Parses `GET https://graph.microsoft.com/v1.0/me`. Microsoft Graph's
/// `/me` never reports an `email_verified`-style claim; `mail` (the
/// mailbox Entra ID actually delivers to) is preferred, falling back to
/// `userPrincipalName` only when `mail` is null — a personal Microsoft
/// account (as opposed to work/school) commonly has no `mail` set.
pub fn parse_microsoft_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: MicrosoftUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("microsoft userinfo: {e}")))?;
    let email = user.mail.filter(|m| !m.is_empty()).unwrap_or(user.user_principal_name);
    Ok(OAuth2User {
        id: user.id,
        name: user.display_name,
        username: String::new(),
        email,
        avatar_url: String::new(),
    })
}

#[derive(Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    verified: bool,
    #[serde(default)]
    avatar: Option<String>,
}

/// Parses `GET https://discord.com/api/users/@me`.
pub fn parse_discord_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: DiscordUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("discord userinfo: {e}")))?;
    let avatar_url = user
        .avatar
        .as_deref()
        .filter(|a| !a.is_empty())
        .map(|a| format!("https://cdn.discordapp.com/avatars/{}/{a}.png", user.id))
        .unwrap_or_default();
    Ok(OAuth2User {
        id: user.id,
        name: user.global_name.unwrap_or_default(),
        username: user.username,
        email: if user.verified { user.email.unwrap_or_default() } else { String::new() },
        avatar_url,
    })
}

#[derive(Deserialize)]
struct GitLabUser {
    id: i64,
    #[serde(default)]
    username: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    confirmed_at: Option<String>,
    #[serde(default)]
    avatar_url: String,
}

/// Parses `GET {baseUrl}/api/v4/user`. `email` is only kept once
/// `confirmed_at` is present — GitLab lets an unconfirmed address sit on
/// an account indefinitely.
pub fn parse_gitlab_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: GitLabUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("gitlab userinfo: {e}")))?;
    let confirmed = user.confirmed_at.is_some();
    Ok(OAuth2User {
        id: user.id.to_string(),
        name: user.name,
        username: user.username,
        email: if confirmed { user.email.unwrap_or_default() } else { String::new() },
        avatar_url: user.avatar_url,
    })
}

#[derive(Deserialize)]
struct FacebookPicture {
    data: FacebookPictureData,
}
#[derive(Deserialize)]
struct FacebookPictureData {
    #[serde(default)]
    url: String,
}
#[derive(Deserialize)]
struct FacebookUser {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    picture: Option<FacebookPicture>,
}

/// Parses `GET https://graph.facebook.com/me?fields=id,name,email,picture...`.
/// Facebook only ever returns a (long-since verified) primary email at
/// all when one exists and the app was granted the `email` permission —
/// there is no separate `email_verified` claim to check.
pub fn parse_facebook_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: FacebookUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("facebook userinfo: {e}")))?;
    Ok(OAuth2User {
        id: user.id,
        name: user.name,
        username: String::new(),
        email: user.email.unwrap_or_default(),
        avatar_url: user.picture.map(|p| p.data.url).unwrap_or_default(),
    })
}

#[derive(Deserialize)]
struct TwitterUserWrapper {
    data: TwitterUser,
}
#[derive(Deserialize)]
struct TwitterUser {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    profile_image_url: String,
}

/// Parses `GET https://api.twitter.com/2/users/me`. X's v2 API has no
/// generally-available email scope, so `email` is always empty — an app
/// wanting one has to fall back to asking for it out of band.
pub fn parse_twitter_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let wrapper: TwitterUserWrapper = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("twitter userinfo: {e}")))?;
    Ok(OAuth2User {
        id: wrapper.data.id,
        name: wrapper.data.name,
        username: wrapper.data.username,
        email: String::new(),
        avatar_url: wrapper.data.profile_image_url,
    })
}

#[derive(Deserialize)]
struct LinkedInUser {
    sub: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    email_verified: Value,
    #[serde(default)]
    picture: String,
}

/// Parses `GET https://api.linkedin.com/v2/userinfo` (LinkedIn's OIDC
/// userinfo endpoint — the `Sign In with LinkedIn using OpenID Connect`
/// product, not the legacy REST API).
pub fn parse_linkedin_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: LinkedInUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("linkedin userinfo: {e}")))?;
    let verified = is_truthy(&user.email_verified);
    Ok(OAuth2User {
        id: user.sub,
        name: user.name,
        username: String::new(),
        email: if verified { user.email } else { String::new() },
        avatar_url: user.picture,
    })
}

#[derive(Deserialize)]
struct SlackUser {
    sub: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    email_verified: Value,
    #[serde(default)]
    picture: String,
}

/// Parses `GET https://slack.com/api/openid.connect.userInfo` (Slack's
/// `Sign in with Slack` OIDC product).
pub fn parse_slack_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: SlackUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("slack userinfo: {e}")))?;
    let verified = is_truthy(&user.email_verified);
    Ok(OAuth2User {
        id: user.sub,
        name: user.name,
        username: String::new(),
        email: if verified { user.email } else { String::new() },
        avatar_url: user.picture,
    })
}

#[derive(Deserialize)]
struct TwitchUserWrapper {
    data: Vec<TwitchUser>,
}
#[derive(Deserialize)]
struct TwitchUser {
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    login: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    profile_image_url: String,
}

/// Parses `GET https://api.twitch.tv/helix/users` — wrapped in a
/// single-element `data` array, Helix's usual response envelope. `email`
/// is present at all only when the `user:read:email` scope was granted,
/// and Twitch requires the account's address to already be verified
/// before it can be used to authorize an app, so no separate check is
/// needed here.
pub fn parse_twitch_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let wrapper: TwitchUserWrapper = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("twitch userinfo: {e}")))?;
    let user = wrapper
        .data
        .into_iter()
        .next()
        .ok_or_else(|| AuthError::InvalidOAuth2Response("twitch userinfo: empty data".into()))?;
    Ok(OAuth2User {
        id: user.id,
        name: user.display_name,
        username: user.login,
        email: user.email.unwrap_or_default(),
        avatar_url: user.profile_image_url,
    })
}

#[derive(Deserialize)]
struct SpotifyImage {
    #[serde(default)]
    url: String,
}
#[derive(Deserialize)]
struct SpotifyUser {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    images: Vec<SpotifyImage>,
}

/// Parses `GET https://api.spotify.com/v1/me`. Spotify does not expose
/// whether the email was ever verified, but only lets one Spotify
/// account claim it as its account email in the first place.
pub fn parse_spotify_userinfo(body: &[u8]) -> AuthResult<OAuth2User> {
    let user: SpotifyUser = serde_json::from_slice(body)
        .map_err(|e| AuthError::InvalidOAuth2Response(format!("spotify userinfo: {e}")))?;
    Ok(OAuth2User {
        id: user.id,
        name: user.display_name.unwrap_or_default(),
        username: String::new(),
        email: user.email.unwrap_or_default(),
        avatar_url: user.images.into_iter().next().map(|i| i.url).unwrap_or_default(),
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
        assert_eq!(KnownProvider::from_name("gitlab"), Some(KnownProvider::GitLab));
        assert_eq!(KnownProvider::from_name("not-a-provider"), None);
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

    // --- table-driven: every preset resolves a full, non-empty set of
    // endpoints + scope + display name, and only Apple has no userinfo
    // endpoint. ---

    fn all_presets() -> Vec<KnownProvider> {
        vec![
            KnownProvider::Google,
            KnownProvider::GitHub,
            KnownProvider::Apple,
            KnownProvider::Microsoft,
            KnownProvider::Discord,
            KnownProvider::GitLab,
            KnownProvider::Facebook,
            KnownProvider::Twitter,
            KnownProvider::LinkedIn,
            KnownProvider::Slack,
            KnownProvider::Twitch,
            KnownProvider::Spotify,
        ]
    }

    #[test]
    fn every_preset_has_https_endpoints_scope_and_display_name() {
        let extra = Map::new();
        for provider in all_presets() {
            let auth_url = provider.auth_url(&extra);
            let token_url = provider.token_url(&extra);
            let user_info_url = provider.user_info_url(&extra);
            assert!(auth_url.starts_with("https://"), "{provider:?} authURL");
            assert!(token_url.starts_with("https://"), "{provider:?} tokenURL");
            if provider == KnownProvider::Apple {
                assert!(user_info_url.is_empty(), "apple has no userinfo endpoint");
            } else {
                assert!(
                    user_info_url.starts_with("https://"),
                    "{provider:?} userInfoURL"
                );
            }
            assert!(!provider.default_scope().is_empty(), "{provider:?} scope");
            assert!(!provider.display_name().is_empty(), "{provider:?} name");
            assert!(provider.supports_pkce(), "{provider:?} pkce");
        }
    }

    #[test]
    fn from_name_round_trips_every_preset_by_its_lowercase_name() {
        let cases = [
            ("google", KnownProvider::Google),
            ("github", KnownProvider::GitHub),
            ("apple", KnownProvider::Apple),
            ("microsoft", KnownProvider::Microsoft),
            ("discord", KnownProvider::Discord),
            ("gitlab", KnownProvider::GitLab),
            ("facebook", KnownProvider::Facebook),
            ("twitter", KnownProvider::Twitter),
            ("linkedin", KnownProvider::LinkedIn),
            ("slack", KnownProvider::Slack),
            ("twitch", KnownProvider::Twitch),
            ("spotify", KnownProvider::Spotify),
        ];
        for (name, expected) in cases {
            assert_eq!(KnownProvider::from_name(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn microsoft_defaults_to_the_common_tenant_but_honors_an_override() {
        let extra = Map::new();
        assert!(KnownProvider::Microsoft
            .auth_url(&extra)
            .contains("/common/"));
        let mut tenant = Map::new();
        tenant.insert("tenant".into(), Value::String("contoso.onmicrosoft.com".into()));
        assert!(KnownProvider::Microsoft
            .auth_url(&tenant)
            .contains("/contoso.onmicrosoft.com/"));
        assert!(KnownProvider::Microsoft
            .token_url(&tenant)
            .contains("/contoso.onmicrosoft.com/"));
    }

    #[test]
    fn gitlab_defaults_to_gitlab_com_but_honors_a_self_hosted_base_url() {
        let extra = Map::new();
        assert_eq!(
            KnownProvider::GitLab.auth_url(&extra),
            "https://gitlab.com/oauth/authorize"
        );
        let mut base = Map::new();
        base.insert(
            "baseUrl".into(),
            Value::String("https://gitlab.example.com/".into()),
        );
        assert_eq!(
            KnownProvider::GitLab.token_url(&base),
            "https://gitlab.example.com/oauth/token"
        );
        assert_eq!(
            KnownProvider::GitLab.user_info_url(&base),
            "https://gitlab.example.com/api/v4/user"
        );
    }

    #[test]
    fn only_twitch_requires_the_client_id_header() {
        for provider in all_presets() {
            assert_eq!(
                provider.requires_client_id_header(),
                provider == KnownProvider::Twitch,
                "{provider:?}"
            );
        }
    }

    #[test]
    fn only_github_has_a_separate_emails_endpoint() {
        for provider in all_presets() {
            assert_eq!(
                provider.emails_url().is_some(),
                provider == KnownProvider::GitHub,
                "{provider:?}"
            );
        }
    }

    #[test]
    fn apple_client_secret_is_an_es256_jwt_with_the_right_claims() {
        // A throwaway P-256 test key (NOT a real Apple key), PKCS#8 PEM.
        let pem = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgevZzL1gdAFr88hb2\n\
OF/2NxApJCzGCEDdfSp6VQO30hyhRANCAAQRWz+jn65BtOMvdyHKcvjBeBSDZH2r\n\
1RTwjmYSi9R/zpBnuQ4EiMnCqfMPWiZqB4QdbAd0E7oH50VpuZ1P087G\n\
-----END PRIVATE KEY-----\n";
        let secret = apple_client_secret(
            "TEAMID1234",
            "KEYID6789",
            pem,
            "com.example.app.service",
            1_700_000_000,
        )
        .unwrap();
        let header = jsonwebtoken::decode_header(&secret).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KEYID6789"));
        // Decode without verifying (this test's throwaway key has no
        // matching public key to verify against) to check the claims.
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.insecure_disable_signature_validation();
        validation.set_audience(&["https://appleid.apple.com"]);
        validation.validate_exp = false;
        let decoded = jsonwebtoken::decode::<serde_json::Value>(
            &secret,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &validation,
        )
        .unwrap();
        assert_eq!(decoded.claims["iss"], "TEAMID1234");
        assert_eq!(decoded.claims["sub"], "com.example.app.service");
        assert_eq!(decoded.claims["aud"], "https://appleid.apple.com");
        assert_eq!(decoded.claims["exp"], 1_700_000_300);
    }

    #[test]
    fn apple_id_token_claims_keep_only_a_verified_email() {
        let body = br#"{"sub":"001.abc123","email":"jo@example.com","email_verified":"true"}"#;
        let user = parse_apple_id_token_claims(body).unwrap();
        assert_eq!(user.id, "001.abc123");
        assert_eq!(user.email, "jo@example.com");

        let body = br#"{"sub":"001.abc123","email":"jo@example.com","email_verified":false}"#;
        let user = parse_apple_id_token_claims(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn apple_first_login_name_is_parsed_once_and_only_once() {
        let field = r#"{"name":{"firstName":"Jo","lastName":"March"},"email":"jo@example.com"}"#;
        assert_eq!(parse_apple_first_login_name(field), "Jo March");
        // Every later login: Apple omits `user` entirely.
        assert_eq!(parse_apple_first_login_name(""), "");
    }

    #[test]
    fn parses_microsoft_userinfo_preferring_mail_over_upn() {
        let body = br#"{"id":"abc","displayName":"Jo March","mail":"jo@contoso.com","userPrincipalName":"jo_upn@contoso.com"}"#;
        let user = parse_microsoft_userinfo(body).unwrap();
        assert_eq!(user.email, "jo@contoso.com");

        let body = br#"{"id":"abc","displayName":"Jo","mail":null,"userPrincipalName":"jo_upn@contoso.com"}"#;
        let user = parse_microsoft_userinfo(body).unwrap();
        assert_eq!(user.email, "jo_upn@contoso.com");
    }

    #[test]
    fn parses_discord_userinfo_and_builds_the_avatar_cdn_url() {
        let body = br#"{"id":"123","username":"jo","global_name":"Jo March","email":"jo@example.com","verified":true,"avatar":"abcd1234"}"#;
        let user = parse_discord_userinfo(body).unwrap();
        assert_eq!(user.id, "123");
        assert_eq!(user.email, "jo@example.com");
        assert_eq!(
            user.avatar_url,
            "https://cdn.discordapp.com/avatars/123/abcd1234.png"
        );
    }

    #[test]
    fn discord_never_attributes_an_unverified_email() {
        let body = br#"{"id":"123","username":"jo","email":"spoofed@example.com","verified":false}"#;
        let user = parse_discord_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_gitlab_userinfo_requiring_confirmation() {
        let body = br#"{"id":1,"username":"jo","name":"Jo","email":"jo@example.com","confirmed_at":"2024-01-01T00:00:00Z","avatar_url":"https://gitlab.com/a.png"}"#;
        let user = parse_gitlab_userinfo(body).unwrap();
        assert_eq!(user.email, "jo@example.com");

        let body = br#"{"id":1,"username":"jo","name":"Jo","email":"jo@example.com","confirmed_at":null}"#;
        let user = parse_gitlab_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_facebook_userinfo() {
        let body = br#"{"id":"1","name":"Jo March","email":"jo@example.com","picture":{"data":{"url":"https://fb.example/a.jpg"}}}"#;
        let user = parse_facebook_userinfo(body).unwrap();
        assert_eq!(user.email, "jo@example.com");
        assert_eq!(user.avatar_url, "https://fb.example/a.jpg");
    }

    #[test]
    fn parses_twitter_userinfo_with_no_email() {
        let body = br#"{"data":{"id":"1","name":"Jo March","username":"jomarch","profile_image_url":"https://x.example/a.jpg"}}"#;
        let user = parse_twitter_userinfo(body).unwrap();
        assert_eq!(user.username, "jomarch");
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_linkedin_userinfo() {
        let body = br#"{"sub":"abc","name":"Jo March","email":"jo@example.com","email_verified":true,"picture":"https://li.example/a.jpg"}"#;
        let user = parse_linkedin_userinfo(body).unwrap();
        assert_eq!(user.id, "abc");
        assert_eq!(user.email, "jo@example.com");
    }

    #[test]
    fn linkedin_never_attributes_an_unverified_email() {
        let body = br#"{"sub":"abc","email":"spoofed@example.com","email_verified":false}"#;
        let user = parse_linkedin_userinfo(body).unwrap();
        assert_eq!(user.email, "");
    }

    #[test]
    fn parses_slack_userinfo() {
        let body = br#"{"sub":"abc","name":"Jo March","email":"jo@example.com","email_verified":true,"picture":"https://slack.example/a.jpg"}"#;
        let user = parse_slack_userinfo(body).unwrap();
        assert_eq!(user.id, "abc");
        assert_eq!(user.email, "jo@example.com");
    }

    #[test]
    fn parses_twitch_userinfo_from_the_data_envelope() {
        let body = br#"{"data":[{"id":"1","display_name":"Jo","login":"jo","email":"jo@example.com","profile_image_url":"https://twitch.example/a.jpg"}]}"#;
        let user = parse_twitch_userinfo(body).unwrap();
        assert_eq!(user.id, "1");
        assert_eq!(user.email, "jo@example.com");
    }

    #[test]
    fn twitch_userinfo_rejects_an_empty_data_array() {
        assert!(parse_twitch_userinfo(br#"{"data":[]}"#).is_err());
    }

    #[test]
    fn parses_spotify_userinfo() {
        let body = br#"{"id":"1","display_name":"Jo March","email":"jo@example.com","images":[{"url":"https://spotify.example/a.jpg"}]}"#;
        let user = parse_spotify_userinfo(body).unwrap();
        assert_eq!(user.email, "jo@example.com");
        assert_eq!(user.avatar_url, "https://spotify.example/a.jpg");
    }
}
