//! Collections, shaped like PocketBase v0.23+'s collection JSON. Base
//! collections carry only the common attributes; view collections add
//! `viewQuery`; auth collections add the flat auth option blocks
//! (`passwordAuth`, `oauth2`, `otp`, `mfa`, `authToken`, templates, ...).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::datetime::DateTime;
use crate::field::{Field, FieldKind, FieldType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CollectionType {
    #[default]
    Base,
    Auth,
    View,
}

impl CollectionType {
    pub fn as_str(self) -> &'static str {
        match self {
            CollectionType::Base => "base",
            CollectionType::Auth => "auth",
            CollectionType::View => "view",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenConfig {
    pub duration: i64,
    /// Optional per-collection signing secret suffix; empty means the
    /// app secret alone (plus the record's `tokenKey`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthAlert {
    pub enabled: bool,
    pub email_template: EmailTemplate,
}

impl Default for AuthAlert {
    fn default() -> Self {
        AuthAlert {
            enabled: true,
            email_template: EmailTemplate {
                subject: "Login from a new location".into(),
                body: "<p>Hello,</p>\n<p>We noticed a login to your {APP_NAME} account from a new location:</p>\n<p><em>{ALERT_INFO}</em></p>\n<p><strong>If this wasn't you, you should immediately change your {APP_NAME} account password to revoke access from all other locations.</strong></p>\n<p>If this was you, you may disregard this email.</p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OAuth2Provider {
    pub name: String,
    pub client_id: String,
    /// Persisted like every other stored secret in this codebase
    /// (`to_json()` is the same value written to `_collections.options`
    /// — see `crates/db/src/collections.rs`'s `row_params`). Stripped
    /// from the *outgoing* API/dashboard response instead, by
    /// `crates/server/src/routes/collections.rs`'s response redaction,
    /// so it still never round-trips back to a caller.
    pub client_secret: String,
    #[serde(rename = "authURL")]
    pub auth_url: String,
    #[serde(rename = "tokenURL")]
    pub token_url: String,
    #[serde(rename = "userInfoURL")]
    pub user_info_url: String,
    pub display_name: String,
    pub pkce: Option<bool>,
    pub extra: Map<String, Value>,
}

impl Default for OAuth2Provider {
    fn default() -> Self {
        OAuth2Provider {
            name: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            auth_url: String::new(),
            token_url: String::new(),
            user_info_url: String::new(),
            display_name: String::new(),
            pkce: None,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OAuth2MappedFields {
    pub id: String,
    pub name: String,
    pub username: String,
    #[serde(rename = "avatarURL")]
    pub avatar_url: String,
}

impl Default for OAuth2MappedFields {
    fn default() -> Self {
        OAuth2MappedFields {
            id: String::new(),
            name: "name".into(),
            username: String::new(),
            avatar_url: "avatar".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OAuth2 {
    pub enabled: bool,
    pub providers: Vec<OAuth2Provider>,
    pub mapped_fields: OAuth2MappedFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PasswordAuth {
    pub enabled: bool,
    pub identity_fields: Vec<String>,
}

impl Default for PasswordAuth {
    fn default() -> Self {
        PasswordAuth {
            enabled: true,
            identity_fields: vec!["email".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mfa {
    pub enabled: bool,
    pub duration: i64,
    pub rule: String,
}

impl Default for Mfa {
    fn default() -> Self {
        Mfa {
            enabled: false,
            duration: 600,
            rule: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Otp {
    pub enabled: bool,
    pub duration: i64,
    pub length: i64,
    pub email_template: EmailTemplate,
}

impl Default for Otp {
    fn default() -> Self {
        Otp {
            enabled: false,
            duration: 180,
            length: 8,
            email_template: EmailTemplate {
                subject: "OTP for {APP_NAME}".into(),
                body: "<p>Hello,</p>\n<p>Your one-time password is: <strong>{OTP}</strong></p>\n<p><i>If you didn't ask for the one-time password, you can ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
            },
        }
    }
}

fn token(duration: i64) -> TokenConfig {
    TokenConfig {
        duration,
        secret: String::new(),
    }
}
fn auth_token_default() -> TokenConfig {
    token(432_000)
}
fn short_token_default() -> TokenConfig {
    token(1_800)
}
fn verification_token_default() -> TokenConfig {
    token(86_400)
}
fn file_token_default() -> TokenConfig {
    token(180)
}
fn empty_rule() -> Option<String> {
    Some(String::new())
}
fn verification_template_default() -> EmailTemplate {
    EmailTemplate {
        subject: "Verify your {APP_NAME} email".into(),
        body: "<p>Hello,</p>\n<p>Thank you for joining us at {APP_NAME}.</p>\n<p>Click on the button below to verify your email address.</p>\n<p>\n  <a class=\"btn\" href=\"{APP_URL}/_/#/auth/confirm-verification/{TOKEN}\" target=\"_blank\" rel=\"noopener\">Verify</a>\n</p>\n<p><i>If you didn't recently register, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
    }
}
fn reset_password_template_default() -> EmailTemplate {
    EmailTemplate {
        subject: "Reset your {APP_NAME} password".into(),
        body: "<p>Hello,</p>\n<p>Click on the button below to reset your password.</p>\n<p>\n  <a class=\"btn\" href=\"{APP_URL}/_/#/auth/confirm-password-reset/{TOKEN}\" target=\"_blank\" rel=\"noopener\">Reset password</a>\n</p>\n<p><i>If you didn't ask to reset your password, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
    }
}
fn confirm_email_change_template_default() -> EmailTemplate {
    EmailTemplate {
        subject: "Confirm your {APP_NAME} new email address".into(),
        body: "<p>Hello,</p>\n<p>Click on the button below to confirm your new email address.</p>\n<p>\n  <a class=\"btn\" href=\"{APP_URL}/_/#/auth/confirm-email-change/{TOKEN}\" target=\"_blank\" rel=\"noopener\">Confirm new email</a>\n</p>\n<p><i>If you didn't ask to change your email address, please ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
    }
}
fn magic_link_url_template_default() -> String {
    "{APP_URL}/auth/magic-link?token={TOKEN}".into()
}
fn magic_link_duration_default() -> i64 {
    900
}
fn magic_link_template_default() -> EmailTemplate {
    EmailTemplate {
        subject: "Sign in to {APP_NAME}".into(),
        body: "<p>Hello,</p>\n<p>Click on the button below to sign in to {APP_NAME}.</p>\n<p>\n  <a class=\"btn\" href=\"{MAGIC_LINK}\" target=\"_blank\" rel=\"noopener\">Sign in</a>\n</p>\n<p><i>If you didn't ask to sign in, you can ignore this email.</i></p>\n<p>\n  Thanks,<br/>\n  {APP_NAME} team\n</p>".into(),
    }
}

/// `authOptions.magicLink` — a passwordless login flow parallel to
/// [`Otp`], but mailing a single-use link instead of a user-typed code.
/// Disabled by default, same as `Mfa`/`Otp`; see
/// `crate::routes::auth`'s `request-magic-link`/`auth-with-magic-link` in
/// the server crate (`_magicLinks` is the token store — same shape as
/// `_otps`, just holding a `tokenHash` instead of a hashed short code).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MagicLink {
    pub enabled: bool,
    #[serde(default = "magic_link_duration_default")]
    pub duration: i64,
    /// The link built when the request body doesn't supply an allow-listed
    /// `redirectUrl`. `{APP_URL}` and `{TOKEN}` are substituted the same
    /// way every other auth template placeholder is.
    #[serde(default = "magic_link_url_template_default")]
    pub url_template: String,
    #[serde(default = "magic_link_template_default")]
    pub email_template: EmailTemplate,
}

impl Default for MagicLink {
    fn default() -> Self {
        MagicLink {
            enabled: false,
            duration: magic_link_duration_default(),
            url_template: magic_link_url_template_default(),
            email_template: magic_link_template_default(),
        }
    }
}

/// Everything specific to `type: "auth"` collections, flattened onto the
/// collection JSON. Defaults are PocketBase's (captured from a freshly
/// created `users` collection), applied per field so a partial input
/// still gets them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthOptions {
    #[serde(default = "empty_rule")]
    pub auth_rule: Option<String>,
    pub manage_rule: Option<String>,
    pub auth_alert: AuthAlert,
    pub oauth2: OAuth2,
    pub password_auth: PasswordAuth,
    pub mfa: Mfa,
    pub otp: Otp,
    #[serde(default = "auth_token_default")]
    pub auth_token: TokenConfig,
    #[serde(default = "short_token_default")]
    pub password_reset_token: TokenConfig,
    #[serde(default = "short_token_default")]
    pub email_change_token: TokenConfig,
    #[serde(default = "verification_token_default")]
    pub verification_token: TokenConfig,
    #[serde(default = "file_token_default")]
    pub file_token: TokenConfig,
    #[serde(default = "verification_template_default")]
    pub verification_template: EmailTemplate,
    #[serde(default = "reset_password_template_default")]
    pub reset_password_template: EmailTemplate,
    #[serde(default = "confirm_email_change_template_default")]
    pub confirm_email_change_template: EmailTemplate,
    pub magic_link: MagicLink,
}

impl Default for AuthOptions {
    fn default() -> Self {
        AuthOptions {
            auth_rule: empty_rule(),
            manage_rule: None,
            auth_alert: AuthAlert::default(),
            oauth2: OAuth2::default(),
            password_auth: PasswordAuth::default(),
            mfa: Mfa::default(),
            otp: Otp::default(),
            auth_token: auth_token_default(),
            password_reset_token: short_token_default(),
            email_change_token: short_token_default(),
            verification_token: verification_token_default(),
            file_token: file_token_default(),
            verification_template: verification_template_default(),
            reset_password_template: reset_password_template_default(),
            confirm_email_change_template: confirm_email_change_template_default(),
            magic_link: MagicLink::default(),
        }
    }
}

/// A collection: persisted in `_collections` and materialized as a real
/// SQL table (or view) named exactly `name`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub collection_type: CollectionType,
    pub system: bool,
    pub fields: Vec<Field>,
    pub indexes: Vec<String>,
    /// `None` = superusers only, `Some("")` = public, `Some(expr)` =
    /// filtered.
    pub list_rule: Option<String>,
    pub view_rule: Option<String>,
    pub create_rule: Option<String>,
    pub update_rule: Option<String>,
    pub delete_rule: Option<String>,
    pub created: DateTime,
    pub updated: DateTime,
    /// View collections only.
    pub view_query: String,
    /// The Postgres text-search config (`"english"`, `"indonesian"`,
    /// ...) used to build the generated `tsvector` column when this
    /// collection has any `searchable` field. `None`/absent falls back
    /// to `"simple"` (no stemming/stopwords — always available, no
    /// extra extension). Ignored on SQLite, which always uses FTS5's
    /// own tokenizer. Not validated against Postgres's actual installed
    /// configs here (that needs a live connection); an unknown name
    /// falls back to `"simple"` at sync time rather than failing the
    /// collection save.
    #[serde(default)]
    pub search_language: Option<String>,
    /// The relation field (pointing at an auth collection) that names a
    /// record's owner, for the `storage.userQuotaBytes` per-user storage
    /// quota (`crates/server/src/quota.rs`) — quota usage is the sum of
    /// every file field's stored size across this collection's records
    /// whose `ownerField` equals the uploading auth record's id. `None`
    /// (the default) means this collection never counts toward or is
    /// gated by the quota, whatever `userQuotaBytes` is set to.
    #[serde(default)]
    pub owner_field: Option<String>,
    /// Auth collections only.
    #[serde(flatten)]
    pub auth: AuthOptions,
}

impl Default for Collection {
    fn default() -> Self {
        Collection {
            id: String::new(),
            name: String::new(),
            collection_type: CollectionType::Base,
            system: false,
            fields: vec![],
            indexes: vec![],
            list_rule: None,
            view_rule: None,
            create_rule: None,
            update_rule: None,
            delete_rule: None,
            created: DateTime::default(),
            updated: DateTime::default(),
            view_query: String::new(),
            search_language: None,
            owner_field: None,
            auth: AuthOptions::default(),
        }
    }
}

impl Serialize for Collection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

impl Collection {
    /// A new, unsaved collection with PocketBase's default system fields
    /// for its type (`id`, plus the auth fields for auth collections).
    /// `created`/`updated` autodate fields are added too, as the scaffold
    /// endpoint does.
    pub fn new(name: impl Into<String>, collection_type: CollectionType) -> Self {
        let name = name.into();
        let mut c = Collection {
            id: crate::ids::collection_id(collection_type.as_str(), &name),
            name,
            collection_type,
            created: DateTime::now(),
            updated: DateTime::now(),
            ..Default::default()
        };
        c.ensure_system_fields();
        if c.fields.iter().all(|f| f.name != "created") {
            c.fields.push(Field::created_field());
        }
        if c.fields.iter().all(|f| f.name != "updated") {
            c.fields.push(Field::updated_field());
        }
        c
    }

    /// Insert the system fields PocketBase guarantees for this type,
    /// keeping any the caller already supplied (by name) but forcing
    /// their system flags. Idempotent.
    pub fn ensure_system_fields(&mut self) {
        let mut required: Vec<Field> = vec![Field::id_field()];
        if self.collection_type == CollectionType::Auth {
            required.extend([
                Field::password_field(),
                Field::token_key_field(),
                Field::email_field(),
                Field::email_visibility_field(),
                Field::verified_field(),
            ]);
        }
        let mut merged: Vec<Field> = Vec::with_capacity(required.len() + self.fields.len());
        for sys in required {
            let existing = self.fields.iter().position(|f| f.name == sys.name);
            match existing {
                Some(pos) => {
                    let mut f = self.fields.remove(pos);
                    f.system = true;
                    f.hidden = sys.hidden;
                    if f.field_type() != sys.field_type() {
                        f.kind = sys.kind.clone();
                    }
                    if f.id.is_empty() {
                        f.id = sys.id.clone();
                    }
                    merged.push(f);
                }
                None => merged.push(sys),
            }
        }
        // `id` first, then the rest in their original order.
        merged.append(&mut self.fields);
        self.fields = merged;
        self.assign_field_ids();
    }

    /// Give every field without an id PocketBase's derived id.
    pub fn assign_field_ids(&mut self) {
        for f in &mut self.fields {
            if f.id.is_empty() {
                f.id = crate::ids::field_id(f.field_type().as_str(), &f.name);
            }
        }
    }

    pub fn is_auth(&self) -> bool {
        self.collection_type == CollectionType::Auth
    }

    pub fn is_view(&self) -> bool {
        self.collection_type == CollectionType::View
    }

    pub fn is_base(&self) -> bool {
        self.collection_type == CollectionType::Base
    }

    pub fn is_superusers(&self) -> bool {
        self.name == crate::SUPERUSERS_COLLECTION
    }

    pub fn is_cron_jobs(&self) -> bool {
        self.name == crate::CRON_JOBS_COLLECTION
    }

    pub fn is_rpc(&self) -> bool {
        self.name == crate::RPC_COLLECTION
    }

    pub fn is_notifications(&self) -> bool {
        self.name == crate::NOTIFICATIONS_COLLECTION
    }

    pub fn is_channels(&self) -> bool {
        self.name == crate::CHANNELS_COLLECTION
    }

    /// A JSON-Schema / OpenAI-function-calling-shaped description of this
    /// collection's writable, non-system fields:
    /// `{name, description, parameters: {type: "object",
    /// properties: {...}, required: [...]}}`. The single source of truth
    /// both the MCP tool definitions and the `/tool-schema` REST endpoint
    /// call, so the two surfaces can never drift apart (see
    /// `crates/server/src/mcp.rs` and
    /// `crates/server/src/routes/tool_schema.rs`). System fields
    /// (`id`, `created`, `updated`, the auth system fields, ...) are
    /// omitted: they are never supplied by a caller.
    pub fn to_json_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for field in self.fields.iter().filter(|f| !f.system) {
            properties.insert(field.name.clone(), field.kind.to_json_schema(&field.help));
            if field.required {
                required.push(Value::String(field.name.clone()));
            }
        }
        let field_count = properties.len();
        json!({
            "name": self.name,
            "description": format!(
                "The '{}' {} collection ({} field{}).",
                self.name,
                self.collection_type.as_str(),
                field_count,
                if field_count == 1 { "" } else { "s" },
            ),
            "parameters": {
                "type": "object",
                "properties": Value::Object(properties),
                "required": required,
            },
        })
    }

    /// The SQL table (or view) name. PocketBase names tables after the
    /// collection so `viewQuery` SQL is portable.
    pub fn table_name(&self) -> &str {
        &self.name
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn field_by_id(&self, id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    pub fn has_field(&self, name: &str) -> bool {
        self.field(name).is_some()
    }

    pub fn fields_of_type(&self, t: FieldType) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(move |f| f.field_type() == t)
    }

    /// The fields that participate in this collection's full-text index,
    /// in schema order (also the column order the FTS5/`tsvector`
    /// expression concatenates them in).
    pub fn searchable_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.is_searchable())
    }

    /// Whether this collection has a full-text index at all.
    pub fn has_search_index(&self) -> bool {
        self.searchable_fields().next().is_some()
    }

    /// The Postgres text-search config to use for this collection's
    /// generated `tsvector`: `search_language` if set, else `"simple"`.
    pub fn search_language_or_default(&self) -> &str {
        self.search_language.as_deref().unwrap_or("simple")
    }

    /// Identity fields usable for password login (auth collections).
    pub fn identity_fields(&self) -> Vec<String> {
        if self.auth.password_auth.identity_fields.is_empty() {
            vec!["email".into()]
        } else {
            self.auth.password_auth.identity_fields.clone()
        }
    }

    /// The PocketBase JSON for this collection: common attributes plus
    /// only the type-specific blocks.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("listRule".into(), json!(self.list_rule));
        m.insert("viewRule".into(), json!(self.view_rule));
        m.insert("createRule".into(), json!(self.create_rule));
        m.insert("updateRule".into(), json!(self.update_rule));
        m.insert("deleteRule".into(), json!(self.delete_rule));
        m.insert("name".into(), json!(self.name));
        m.insert("type".into(), json!(self.collection_type));
        m.insert("fields".into(), json!(self.fields));
        m.insert("indexes".into(), json!(self.indexes));
        m.insert("created".into(), json!(self.created));
        m.insert("updated".into(), json!(self.updated));
        m.insert("system".into(), json!(self.system));
        m.insert("searchLanguage".into(), json!(self.search_language));
        m.insert("ownerField".into(), json!(self.owner_field));
        match self.collection_type {
            CollectionType::View => {
                m.insert("viewQuery".into(), json!(self.view_query));
            }
            CollectionType::Auth => {
                if let Value::Object(auth) = serde_json::to_value(&self.auth).unwrap() {
                    m.extend(auth);
                }
            }
            CollectionType::Base => {}
        }
        Value::Object(m)
    }

    /// Default `posts`-style scaffold for the dashboard's "new
    /// collection" dialog, per type.
    pub fn scaffold(collection_type: CollectionType) -> Self {
        let mut c = Collection::new("", collection_type);
        c.id = String::new();
        c.created = DateTime::default();
        c.updated = DateTime::default();
        if collection_type == CollectionType::Auth {
            c.list_rule = Some("id = @request.auth.id".into());
            c.view_rule = Some("id = @request.auth.id".into());
            c.create_rule = Some(String::new());
            c.update_rule = Some("id = @request.auth.id".into());
            c.delete_rule = Some("id = @request.auth.id".into());
        }
        c
    }

    /// The built-in `users` collection PocketBase seeds on first run.
    pub fn default_users() -> Self {
        let mut c = Collection::new("users", CollectionType::Auth);
        c.id = crate::USERS_COLLECTION_ID.into();
        c.list_rule = Some("id = @request.auth.id".into());
        c.view_rule = Some("id = @request.auth.id".into());
        c.create_rule = Some(String::new());
        c.update_rule = Some("id = @request.auth.id".into());
        c.delete_rule = Some("id = @request.auth.id".into());
        let mut name = Field::new(
            "name",
            FieldKind::Text {
                min: 0,
                max: 255,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        );
        name.id = crate::ids::field_id("text", "name");
        let avatar = Field::new(
            "avatar",
            FieldKind::File {
                max_select: 1,
                max_size: 0,
                mime_types: vec![
                    "image/jpeg".into(),
                    "image/png".into(),
                    "image/svg+xml".into(),
                    "image/gif".into(),
                    "image/webp".into(),
                ],
                thumbs: vec![],
                protected: false,
            },
        );
        // Insert before created/updated.
        let pos = c.fields.len() - 2;
        c.fields.insert(pos, name);
        c.fields.insert(pos + 1, avatar);
        c.indexes = vec![
            format!(
                "CREATE UNIQUE INDEX `idx_tokenKey_{}` ON `users` (`tokenKey`)",
                c.id
            ),
            format!(
                "CREATE UNIQUE INDEX `idx_email_{}` ON `users` (`email`) WHERE `email` != ''",
                c.id
            ),
        ];
        c
    }

    /// The built-in `_superusers` auth collection. `role` (`"owner"` /
    /// `"admin"`, see `Field::role_field`) is what lets more than one
    /// superuser exist with different trust levels — see
    /// `crates/server/src/extract.rs`'s `RequireOwner` and
    /// `crates/server/src/routes/records.rs`'s `_superusers`-specific
    /// write guards for the enforcement side.
    pub fn default_superusers() -> Self {
        let mut c = Collection::new(crate::SUPERUSERS_COLLECTION, CollectionType::Auth);
        c.system = true;
        c.auth.manage_rule = None;
        // Insert before created/updated, same convention `default_users`
        // uses for its own extra fields.
        let pos = c.fields.len() - 2;
        c.fields.insert(pos, Field::role_field());
        c.indexes = vec![
            format!(
                "CREATE UNIQUE INDEX `idx_tokenKey_{}` ON `_superusers` (`tokenKey`)",
                c.id
            ),
            format!(
                "CREATE UNIQUE INDEX `idx_email_{}` ON `_superusers` (`email`) WHERE `email` != ''",
                c.id
            ),
        ];
        c
    }

    /// The system `_externalAuths`, `_mfas`, `_otps`, `_authOrigins`,
    /// `_cron_jobs`, `_rpc`, `_webhooks`, `_teams`, `_team_members` base
    /// collections, with PocketBase's rules and indexes (`_cron_jobs`,
    /// `_rpc`, `_webhooks`, `_teams` and `_team_members` have no
    /// PocketBase equivalent — see their own comments).
    pub fn default_system_collections() -> Vec<Self> {
        let text = |name: &str| {
            let mut f = Field::new(
                name,
                FieldKind::Text {
                    min: 0,
                    max: 0,
                    pattern: String::new(),
                    autogenerate_pattern: String::new(),
                    primary_key: false,
                },
            );
            f.system = true;
            f.required = true;
            f
        };
        let owner_rule = Some(
            "@request.auth.id != '' && recordRef = @request.auth.id && collectionRef = @request.auth.collectionId"
                .to_string(),
        );

        let mut external = Collection::new("_externalAuths", CollectionType::Base);
        external.system = true;
        external.list_rule = owner_rule.clone();
        external.view_rule = owner_rule.clone();
        external.delete_rule = owner_rule.clone();
        let pos = external.fields.len() - 2;
        external.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                text("provider"),
                text("providerId"),
            ],
        );
        external.indexes = vec![
            "CREATE UNIQUE INDEX `idx_externalAuths_record_provider` ON `_externalAuths` (collectionRef, recordRef, provider)".into(),
            "CREATE UNIQUE INDEX `idx_externalAuths_collection_provider` ON `_externalAuths` (collectionRef, provider, providerId)".into(),
        ];

        let mut mfas = Collection::new("_mfas", CollectionType::Base);
        mfas.system = true;
        mfas.list_rule = owner_rule.clone();
        mfas.view_rule = owner_rule.clone();
        let pos = mfas.fields.len() - 2;
        mfas.fields.splice(
            pos..pos,
            [text("collectionRef"), text("recordRef"), text("method")],
        );
        mfas.indexes = vec![
            "CREATE INDEX `idx_mfas_collectionRef_recordRef` ON `_mfas` (collectionRef,recordRef)"
                .into(),
        ];

        let mut otps = Collection::new("_otps", CollectionType::Base);
        otps.system = true;
        otps.list_rule = owner_rule.clone();
        otps.view_rule = owner_rule.clone();
        let mut password = Field::password_field();
        password.kind = FieldKind::Password {
            min: 0,
            max: 0,
            pattern: String::new(),
            cost: 8,
        };
        let mut sent_to = text("sentTo");
        sent_to.required = false;
        sent_to.hidden = true;
        let pos = otps.fields.len() - 2;
        otps.fields.splice(
            pos..pos,
            [text("collectionRef"), text("recordRef"), password, sent_to],
        );
        otps.indexes = vec![
            "CREATE INDEX `idx_otps_collectionRef_recordRef` ON `_otps` (collectionRef, recordRef)"
                .into(),
        ];

        // TOTP 2FA state: at most one row per `(collectionRef, recordRef)`
        // pair, created `pending` (unconfirmed) by `.../totp/setup` and
        // flipped to `confirmed` by `.../totp/confirm` — see
        // `crate::routes::totp` in the server crate. `secret` is
        // encrypted with `CB_ENCRYPTION` when set (same `ParamCipher` as
        // `_params`, applied by the server route rather than at the
        // storage layer, since only that route ever needs the plaintext
        // back). `backupCodes` is a JSON array of hashed one-time codes
        // (SHA-256 hex, like `_otps`/`_magicLinks`'s own codes/tokens) —
        // an entry is removed from the array the moment it's consumed,
        // so "already used" needs no separate flag. `lastUsedStep` is
        // the TOTP replay guard (`cratebase_auth::verify_totp`'s
        // `last_used_step`): a 30-second step at or before this value is
        // never accepted again, even if it's still numerically valid.
        // Mutations only ever go through the dedicated TOTP routes
        // (same reasoning as `_sessions`/`_bans` above), so
        // create/update/delete stay superuser-only.
        let mut totps = Collection::new("_totps", CollectionType::Base);
        totps.system = true;
        totps.list_rule = owner_rule.clone();
        totps.view_rule = owner_rule.clone();
        let mut t_secret = text("secret");
        t_secret.hidden = true;
        let mut t_confirmed = Field::new("confirmed", FieldKind::Bool {});
        t_confirmed.system = true;
        let mut t_backup_codes = Field::new("backupCodes", FieldKind::Json { max_size: 0 });
        t_backup_codes.required = false;
        t_backup_codes.hidden = true;
        let mut t_last_used_step =
            Field::new("lastUsedStep", FieldKind::default_for(FieldType::Number));
        t_last_used_step.system = true;
        t_last_used_step.required = false;
        let pos = totps.fields.len() - 2;
        totps.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                t_secret,
                t_confirmed,
                t_backup_codes,
                t_last_used_step,
            ],
        );
        totps.indexes = vec![
            "CREATE UNIQUE INDEX `idx_totps_unique_pairs` ON `_totps` (collectionRef, recordRef)"
                .into(),
        ];

        // Token store for `authOptions.magicLink` (`crate::routes::auth`'s
        // `request-magic-link`/`auth-with-magic-link` in the server
        // crate), same shape and trust tier as `_otps` immediately above
        // — the only difference is a hashed opaque `tokenHash` (looked up
        // by exact match, like `_sessions.tokenHash`) in place of a
        // hashed short code looked up by `otpId`, since a magic link's
        // token travels in a URL rather than being typed back in
        // alongside a separate id.
        let mut magic_links = Collection::new("_magicLinks", CollectionType::Base);
        magic_links.system = true;
        magic_links.list_rule = owner_rule.clone();
        magic_links.view_rule = owner_rule.clone();
        let mut ml_token_hash = text("tokenHash");
        ml_token_hash.hidden = true;
        let mut ml_sent_to = text("sentTo");
        ml_sent_to.required = false;
        ml_sent_to.hidden = true;
        let mut ml_redirect_url = text("redirectUrl");
        ml_redirect_url.required = false;
        let pos = magic_links.fields.len() - 2;
        magic_links.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                ml_token_hash,
                ml_sent_to,
                ml_redirect_url,
            ],
        );
        magic_links.indexes = vec![
            "CREATE UNIQUE INDEX `idx_magicLinks_tokenHash` ON `_magicLinks` (tokenHash)".into(),
            "CREATE INDEX `idx_magicLinks_collectionRef_recordRef` ON `_magicLinks` (collectionRef, recordRef)".into(),
        ];

        let mut origins = Collection::new("_authOrigins", CollectionType::Base);
        origins.system = true;
        origins.list_rule = owner_rule.clone();
        origins.view_rule = owner_rule.clone();
        origins.delete_rule = owner_rule.clone();
        let pos = origins.fields.len() - 2;
        origins.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                text("fingerprint"),
            ],
        );
        origins.indexes = vec![
            "CREATE UNIQUE INDEX `idx_authOrigins_unique_pairs` ON `_authOrigins` (collectionRef, recordRef, fingerprint)".into(),
        ];

        // Session ledger: one row per minted auth token (password/otp/
        // oauth2/impersonation/refresh), letting a session be listed and
        // revoked without touching the stateless token verification path
        // (`crates/server/src/extract.rs`). `tokenHash` is `sha256(token)`,
        // never the raw token — session identity is derived the same way
        // an incoming request re-derives it, so nothing here can be used
        // to forge a session. `list_rule`/`view_rule` let a record see its
        // own sessions; `create_rule`/`update_rule`/`delete_rule` stay
        // `None` (superuser-only) because the only intended mutation path
        // is `crates/server/src/sessions.rs`'s own functions, called from
        // the auth/session routes — not a second, rule-driven write path
        // that would have to be kept in sync with them.
        let mut sessions = Collection::new("_sessions", CollectionType::Base);
        sessions.system = true;
        sessions.list_rule = owner_rule.clone();
        sessions.view_rule = owner_rule.clone();
        let mut s_token_hash = text("tokenHash");
        s_token_hash.hidden = true;
        let s_kind = text("kind");
        let s_fingerprint = text("fingerprint");
        let mut s_ip = text("ip");
        s_ip.required = false;
        let mut s_user_agent = text("userAgent");
        s_user_agent.required = false;
        let mut s_expires_at = Field::new(
            "expiresAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        s_expires_at.system = true;
        s_expires_at.required = true;
        let mut s_last_seen_at = Field::new(
            "lastSeenAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        s_last_seen_at.system = true;
        s_last_seen_at.required = false;
        let mut s_revoked = Field::new("revoked", FieldKind::Bool {});
        s_revoked.system = true;
        let pos = sessions.fields.len() - 2;
        sessions.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                s_token_hash,
                s_kind,
                s_fingerprint,
                s_ip,
                s_user_agent,
                s_expires_at,
                s_last_seen_at,
                s_revoked,
            ],
        );
        sessions.indexes = vec![
            "CREATE UNIQUE INDEX `idx_sessions_token` ON `_sessions` (tokenHash)".into(),
            "CREATE INDEX `idx_sessions_record` ON `_sessions` (collectionRef, recordRef)".into(),
        ];

        // Bans: superuser-only end to end (never listable/viewable by
        // the banned record itself, unlike `_sessions`/`_authOrigins` —
        // a banned user has no reason to see why). `active_ban` in
        // `crates/server/src/routes/session.rs` reads this table
        // directly; there is no rule-driven path a client could use to
        // ban/unban itself.
        let mut bans = Collection::new("_bans", CollectionType::Base);
        bans.system = true;
        let mut b_reason = text("reason");
        b_reason.required = false;
        let mut b_expires_at = Field::new(
            "expiresAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        b_expires_at.system = true;
        b_expires_at.required = false;
        let mut b_banned_by = text("bannedBy");
        b_banned_by.system = true;
        b_banned_by.required = false;
        let pos = bans.fields.len() - 2;
        bans.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                b_reason,
                b_expires_at,
                b_banned_by,
            ],
        );
        bans.indexes = vec![
            "CREATE UNIQUE INDEX `idx_bans_unique_pairs` ON `_bans` (collectionRef, recordRef)"
                .into(),
        ];

        // Superuser-only end to end (list/view/create/update/delete all
        // stay at `Collection::new`'s default `None`) — a custom cron
        // job runs arbitrary SQL on a schedule with no rule enforcement
        // in between, the same trust tier as editing a collection's
        // schema or the dashboard's SQL console, not something any
        // non-superuser record should ever reach.
        let mut cron_jobs = Collection::new("_cron_jobs", CollectionType::Base);
        cron_jobs.system = true;
        let mut enabled = Field::new("enabled", FieldKind::Bool {});
        enabled.system = true;
        let mut last_run_at = Field::new(
            "lastRunAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        last_run_at.system = true;
        last_run_at.required = false;
        let mut last_status = text("lastStatus");
        last_status.required = false;
        let mut last_message = text("lastMessage");
        last_message.required = false;
        let pos = cron_jobs.fields.len() - 2;
        cron_jobs.fields.splice(
            pos..pos,
            [
                text("name"),
                text("expression"),
                text("sql"),
                enabled,
                last_run_at,
                last_status,
                last_message,
            ],
        );

        // Superuser-only end to end, same trust tier as `_cron_jobs` just
        // above: a definition's `sql` is arbitrary SQL, gated at *call*
        // time (`POST /api/rpc/{name}`) by its own `rule` field, not by a
        // rule here — managing the definitions themselves (like managing
        // cron jobs) is an operator action, not something any
        // non-superuser record should ever reach.
        //
        // `rule`/`params` are `json`, not `text`: the generic column
        // encoder normalizes a plain `text` field's JSON `null` to `''`
        // (see `crates/db/src/records.rs`'s `column_value`), which would
        // erase the `rule = null` ("superuser only") / `rule = ""`
        // ("anyone") distinction the RPC endpoint's rule semantics depend
        // on. A `json` field keeps `null` and `""` distinct end to end.
        let mut rpc = Collection::new(crate::RPC_COLLECTION, CollectionType::Base);
        rpc.system = true;
        let mut rpc_params = Field::new("params", FieldKind::Json { max_size: 0 });
        rpc_params.required = false;
        let mut rpc_rule = Field::new("rule", FieldKind::Json { max_size: 0 });
        rpc_rule.required = false;
        let mut rpc_read_only = Field::new("readOnly", FieldKind::Bool {});
        rpc_read_only.required = false;
        let mut rpc_timeout_ms = Field::new("timeoutMs", FieldKind::default_for(FieldType::Number));
        rpc_timeout_ms.required = false;
        let mut rpc_max_rows = Field::new("maxRows", FieldKind::default_for(FieldType::Number));
        rpc_max_rows.required = false;
        let pos = rpc.fields.len() - 2;
        rpc.fields.splice(
            pos..pos,
            [
                text("name"),
                text("sql"),
                rpc_params,
                rpc_rule,
                rpc_read_only,
                rpc_timeout_ms,
                rpc_max_rows,
            ],
        );
        rpc.indexes = vec![format!(
            "CREATE UNIQUE INDEX `idx_rpc_name_{}` ON `_rpc` (`name`)",
            rpc.id
        )];

        // Superuser-only end to end, same reasoning as `_cron_jobs` above:
        // a webhook's `url`/`secret` are operator-configured integration
        // points, not something a non-superuser record should ever read
        // or edit. `_webhooks` itself is deliberately never a valid
        // `collectionRef` target (see `crate::webhooks` in the server
        // crate) so a write to a webhook row can never re-trigger a
        // webhook dispatch for `_webhooks` writes.
        let mut webhooks = Collection::new("_webhooks", CollectionType::Base);
        webhooks.system = true;
        let mut w_enabled = Field::new("enabled", FieldKind::Bool {});
        w_enabled.system = true;
        w_enabled.required = true;
        let mut w_secret = text("secret");
        w_secret.required = false;
        w_secret.hidden = true;
        let mut w_last_triggered_at = Field::new(
            "lastTriggeredAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        w_last_triggered_at.system = true;
        w_last_triggered_at.required = false;
        let mut w_last_status = text("lastStatus");
        w_last_status.required = false;
        let mut w_last_message = text("lastMessage");
        w_last_message.required = false;
        // Per-webhook override for how many delivery attempts
        // `crate::webhook_deliveries` (server crate) makes before giving up
        // and leaving a `_webhookDeliveries` row `failed`; `0`/absent means
        // "use the global default" (6) — see that module's doc.
        let mut w_max_attempts = Field::new(
            "maxAttempts",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        w_max_attempts.required = false;
        // How many *consecutive* failed deliveries this webhook has had in
        // a row; reset to `0` the moment one succeeds. `crate::webhook_deliveries`
        // flips `enabled` to `false` (and logs an `_audit_log` row) once
        // this reaches the default-50 auto-disable threshold, so a
        // permanently-broken endpoint stops burning delivery attempts
        // forever.
        let mut w_consecutive_failures = Field::new(
            "consecutiveFailures",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        w_consecutive_failures.system = true;
        let pos = webhooks.fields.len() - 2;
        webhooks.fields.splice(
            pos..pos,
            [
                text("name"),
                text("collectionRef"),
                text("events"),
                text("url"),
                w_secret,
                w_enabled,
                w_last_triggered_at,
                w_last_status,
                w_last_message,
                w_max_attempts,
                w_consecutive_failures,
            ],
        );

        // Ordinary application-level multi-user workspaces (a Slack
        // workspace shape), *not* a way to reach the admin dashboard —
        // membership in `_team_members` says nothing about superuser
        // access. `_teams` is listable/viewable by its own members via a
        // `@collection._team_members` back-reference (see
        // `crate::teams`'s module doc in the server crate for the exact
        // pattern any third-party collection should copy to scope itself
        // to a team). Creating a team is open to any authenticated user,
        // who must submit `ownerRef` as their own id; `crate::teams`'s
        // reactive hook then inserts the first `_team_members` owner row
        // so a team is never left ownerless. Update/delete stay
        // superuser-only (rule `None`, `Collection::new`'s default) for
        // this pass.
        let team_member_rule = Some(
            "@collection._team_members.userRef ?= @request.auth.id && @collection._team_members.teamRef ?= id"
                .to_string(),
        );
        let mut teams = Collection::new("_teams", CollectionType::Base);
        teams.system = true;
        teams.list_rule = team_member_rule.clone();
        teams.view_rule = team_member_rule;
        teams.create_rule =
            Some("@request.auth.id != '' && ownerRef = @request.auth.id".to_string());
        let mut owner_ref = Field::new(
            "ownerRef",
            FieldKind::Relation {
                collection_id: crate::USERS_COLLECTION_ID.into(),
                cascade_delete: false,
                min_select: 0,
                max_select: 1,
            },
        );
        owner_ref.required = true;
        let pos = teams.fields.len() - 2;
        teams.fields.splice(pos..pos, [text("name"), owner_ref]);

        // Membership rows: listable/viewable by any member of the same
        // team (a self-referencing `@collection._team_members` check —
        // the joined row need not be *this* row, just some row proving
        // the caller belongs to the same `teamRef`), but only creatable
        // by the team's own owner. The very first, bootstrap owner row
        // for a brand-new team is inserted directly by
        // `crate::teams::bind_hooks`, not through this rule — a team has
        // no owner row yet at the moment it is created, so no caller
        // could ever satisfy `create_rule` for it.
        let same_team_rule = Some(
            "@collection._team_members.userRef ?= @request.auth.id && @collection._team_members.teamRef ?= teamRef"
                .to_string(),
        );
        let mut team_members = Collection::new("_team_members", CollectionType::Base);
        team_members.system = true;
        team_members.list_rule = same_team_rule.clone();
        team_members.view_rule = same_team_rule;
        team_members.create_rule = Some(
            "@collection._team_members.userRef ?= @request.auth.id && @collection._team_members.teamRef ?= teamRef && @collection._team_members.role ?= 'owner'"
                .to_string(),
        );
        let mut team_ref = Field::new(
            "teamRef",
            FieldKind::Relation {
                collection_id: teams.id.clone(),
                cascade_delete: true,
                min_select: 0,
                max_select: 1,
            },
        );
        team_ref.required = true;
        let mut user_ref = Field::new(
            "userRef",
            FieldKind::Relation {
                collection_id: crate::USERS_COLLECTION_ID.into(),
                cascade_delete: true,
                min_select: 0,
                max_select: 1,
            },
        );
        user_ref.required = true;
        let pos = team_members.fields.len() - 2;
        team_members
            .fields
            .splice(pos..pos, [team_ref, user_ref, text("role")]);
        team_members.indexes = vec![
            "CREATE UNIQUE INDEX `idx_team_members_unique_pairs` ON `_team_members` (teamRef, userRef)".into(),
        ];

        // Superuser-only end to end, same trust tier as `_cron_jobs` and
        // `_webhooks` above: a per-request token ledger is an operator
        // audit trail, not something a non-superuser record should ever
        // read (it would leak other callers' usage) or write (a forged
        // row would corrupt the ledger). See `crate::llm` in the server
        // crate for the writer — a best-effort insert after each
        // `POST /api/llm/chat` completes, bypassing this rule the same
        // way `_cron_jobs`'s status write-back bypasses its own.
        let mut llm_usage = Collection::new("_llm_usage", CollectionType::Base);
        llm_usage.system = true;
        let mut caller_id = text("callerId");
        caller_id.required = false;
        let model = text("model");
        let mut prompt_tokens = Field::new(
            "promptTokens",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        prompt_tokens.system = true;
        let mut completion_tokens = Field::new(
            "completionTokens",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        completion_tokens.system = true;
        let pos = llm_usage.fields.len() - 2;
        llm_usage.fields.splice(
            pos..pos,
            [caller_id, model, prompt_tokens, completion_tokens],
        );
        // Superuser-only end to end, same trust tier as `_cron_jobs`/
        // `_webhooks`/`_llm_usage` above: an API key is a first-class
        // identity a superuser mints for a script/agent/MCP client, not
        // something a non-superuser record should ever list (it would
        // leak other callers' key material) or write. `key` stores a
        // salted hash, never the raw key — see `crate::api_keys` in the
        // server crate for the extractor that hashes an incoming
        // `Authorization: Bearer <key>` header the same way a password
        // is checked, and for the one-time plaintext response on
        // creation. `prefix` is the first 8 characters of the raw key,
        // stored in the clear so the dashboard can show "cb_a1b2c3d4…"
        // for identification without ever re-displaying the full value.
        //
        // `actsAsCollection`/`actsAsRecord` are the scoping pair: when
        // both are set, they name a real record in a real auth
        // collection and `crate::api_keys::resolve` builds the exact
        // `Auth` a normal login for that record would produce — same
        // collection, same rule context, same `@request.auth.*` — so
        // the key is subject to that record's own rules like anyone
        // else, not a synthetic superuser. Left empty (the default),
        // the key is unscoped root, identical to every key minted
        // before this pair existed. This is deliberately not a second
        // permission system: a scoped key has no rule-evaluation
        // behavior of its own, it just points `resolve` at whose rules
        // to run.
        let mut api_keys = Collection::new("_api_keys", CollectionType::Base);
        api_keys.system = true;
        let mut ak_key = text("key");
        ak_key.system = true;
        ak_key.hidden = true;
        let mut ak_prefix = text("prefix");
        ak_prefix.system = true;
        let mut ak_enabled = Field::new("enabled", FieldKind::Bool {});
        ak_enabled.system = true;
        let mut ak_last_used_at = Field::new(
            "lastUsedAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        ak_last_used_at.system = true;
        ak_last_used_at.required = false;
        let mut ak_acts_as_collection = text("actsAsCollection");
        ak_acts_as_collection.system = true;
        ak_acts_as_collection.required = false;
        let mut ak_acts_as_record = text("actsAsRecord");
        ak_acts_as_record.system = true;
        ak_acts_as_record.required = false;
        let pos = api_keys.fields.len() - 2;
        api_keys.fields.splice(
            pos..pos,
            [
                text("name"),
                ak_key,
                ak_prefix,
                ak_enabled,
                ak_last_used_at,
                ak_acts_as_collection,
                ak_acts_as_record,
            ],
        );
        api_keys.indexes =
            vec!["CREATE UNIQUE INDEX `idx_api_keys_key` ON `_api_keys` (key)".into()];

        // Self-service like `_externalAuths`/`_mfas`/`_otps`: a record
        // registers its own device token and can list/view/delete only
        // its own rows. Unlike those three, registration is a normal
        // client-initiated REST create (there is no separate auth flow
        // that would insert on the caller's behalf), so `create_rule`
        // is the owner rule too, not the superuser-only default. `token`
        // is the opaque platform delivery token (a Web Push endpoint
        // URL, an FCM registration token, or an APNs device token) —
        // hidden because it is a bearer credential for sending that
        // device a notification, not display data.
        let mut push_subscriptions = Collection::new("_push_subscriptions", CollectionType::Base);
        push_subscriptions.system = true;
        push_subscriptions.list_rule = owner_rule.clone();
        push_subscriptions.view_rule = owner_rule.clone();
        push_subscriptions.create_rule = owner_rule.clone();
        push_subscriptions.delete_rule = owner_rule.clone();
        let mut ps_token = text("token");
        ps_token.hidden = true;
        let mut ps_enabled = Field::new("enabled", FieldKind::Bool {});
        ps_enabled.system = true;
        let pos = push_subscriptions.fields.len() - 2;
        push_subscriptions.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                text("platform"),
                ps_token,
                ps_enabled,
            ],
        );
        push_subscriptions.indexes = vec![
            "CREATE UNIQUE INDEX `idx_push_subscriptions_token` ON `_push_subscriptions` (token)"
                .into(),
        ];

        // Append-only: `list_rule`/`view_rule` stay `Collection::new`'s
        // default `None` (superuser-only — this log is itself sensitive,
        // since it can show who else has superuser access), and
        // `create_rule` stays `None` too (matching `_cron_jobs`/
        // `_webhooks`/`_llm_usage`: writes are meant to come from
        // `crate::audit` in the server crate, not a client, though
        // nothing stops a superuser from writing one directly). There is
        // deliberately no way to express "no update/delete, ever, not
        // even for a superuser" through a rule string — `None` still
        // lets a superuser through, same as every other rule here.
        // `crate::audit::bind_hooks` enforces that half fresh, with a
        // pair of `on_record_update`/`on_record_delete` handlers tagged
        // to this collection that reject the write outright before
        // calling `e.next()` — the same "a handler that never calls
        // `next()` replaces the built-in behaviour" pattern
        // `crates/server/src/hooks.rs`'s module doc describes, just
        // applied to *always* refuse rather than conditionally allow.
        // `actor` is nullable because not every audited action has a
        // human behind it (a future system-initiated write, e.g. from a
        // cron job, would leave it unset).
        let mut audit_log = Collection::new("_audit_log", CollectionType::Base);
        audit_log.system = true;
        let mut actor = Field::new(
            "actor",
            FieldKind::Relation {
                collection_id: crate::ids::collection_id("auth", crate::SUPERUSERS_COLLECTION),
                cascade_delete: false,
                min_select: 0,
                max_select: 1,
            },
        );
        actor.required = false;
        actor.system = true;
        let mut action = text("action");
        action.system = true;
        let mut target = text("target");
        target.system = true;
        let mut meta = Field::new("meta", FieldKind::Json { max_size: 0 });
        meta.required = false;
        meta.system = true;
        let pos = audit_log.fields.len() - 2;
        audit_log
            .fields
            .splice(pos..pos, [actor, action, target, meta]);
        audit_log.indexes = vec![
            "CREATE INDEX `idx_audit_log_action` ON `_audit_log` (action)".into(),
            "CREATE INDEX `idx_audit_log_created` ON `_audit_log` (created)".into(),
        ];

        // The editable template store `crate::routes::mails` (server
        // crate) resolves against: `key` (e.g. `auth.verification`,
        // `welcome`) plus an optional `locale`, unique together so one
        // key can have a row per locale. Superuser-only end to end, same
        // tier as `_cron_jobs`/`_webhooks` — a template's `html` is
        // rendered and mailed verbatim, no different in trust terms from
        // a cron job's `sql` or a webhook's `url`. `html` is an `Editor`
        // field (not `Text`) purely so the dashboard's future template
        // editor gets a rich-text widget for free; the server never
        // interprets it as anything but a `{{var}}` template string. See
        // `crates/mailer/src/template.rs`'s `TemplateDoc`.
        let mut email_templates = Collection::new("_emailTemplates", CollectionType::Base);
        email_templates.system = true;
        let mut et_html = Field::new(
            "html",
            FieldKind::Editor {
                max_size: 0,
                convert_urls: false,
            },
        );
        et_html.required = false;
        let mut et_text = text("text");
        et_text.required = false;
        let mut et_locale = text("locale");
        et_locale.required = false;
        let mut et_layout = Field::new("layout", FieldKind::Bool {});
        et_layout.required = false;
        let mut et_description = text("description");
        et_description.required = false;
        let pos = email_templates.fields.len() - 2;
        email_templates.fields.splice(
            pos..pos,
            [
                text("key"),
                text("name"),
                text("subject"),
                et_html,
                et_text,
                et_locale,
                et_layout,
                et_description,
            ],
        );
        // `sendRule` gates non-superuser access to `POST /api/mails/send`
        // for this template (see `crate::mail_templates`/`crate::mails`
        // in the server crate): `null` (the column's own SQL default —
        // see `column_default`) means superuser/API-key only, `""` means
        // any caller may send it, and anything else is a filter-rule
        // expression evaluated per recipient. It is a `Json`-kind field
        // rather than `Text` purely so the column keeps `NULL` and `""`
        // distinct — a plain `Text` field normalizes both to `''`
        // (see `crates/db/src/schema.rs`'s `column_default` doc), which
        // would make "superuser only" and "anyone" indistinguishable.
        // `crate::routes::records` (server crate) special-cases writing
        // an empty-string `sendRule` for this one collection so the
        // dangerous "anyone" value round-trips correctly through the
        // generic records API — see that module's comment.
        let mut et_send_rule = Field::new("sendRule", FieldKind::Json { max_size: 0 });
        et_send_rule.required = false;
        // Opaque visual-editor document (`@react-email/editor`'s JSON,
        // dashboard-only) the server never reads — it always sends from
        // the `html`/`text` columns, which the editor writes back to on
        // save. `null` for a template authored/edited as raw HTML.
        let mut et_design = Field::new("design", FieldKind::Json { max_size: 0 });
        et_design.required = false;
        // Which editor the dashboard opens this template in; the server
        // never reads this either (see `et_design` above) — it only
        // steers the dashboard's own UI.
        let mut et_editor = Field::new(
            "editor",
            FieldKind::Select {
                values: vec!["visual".into(), "html".into()],
                max_select: 1,
            },
        );
        et_editor.required = false;
        let pos = email_templates.fields.len() - 2;
        email_templates
            .fields
            .splice(pos..pos, [et_send_rule, et_design, et_editor]);
        email_templates.indexes = vec![
            "CREATE UNIQUE INDEX `idx_emailTemplates_key_locale` ON `_emailTemplates` (key, locale)".into(),
        ];

        // Send log for `POST /api/mails/send`/JS `$mails.send` — append-
        // only and superuser-read-only, same reasoning as `_audit_log`
        // immediately above (list/view stay `None`; create/update/delete
        // stay `None` too since the only writer is `crate::routes::mails`
        // itself, not a rule-driven client path). `to` is a JSON array of
        // `{address, name}` recipients rather than a single text column,
        // since one send can fan out to several. `retention` cleanup is
        // `settings.logs.mailLogMaxDays`, mirroring `_logs`.
        let mut mail_log = Collection::new("_mailLog", CollectionType::Base);
        mail_log.system = true;
        let mut ml_to = Field::new("to", FieldKind::Json { max_size: 0 });
        ml_to.system = true;
        let mut ml_subject = text("subject");
        ml_subject.required = false;
        let mut ml_template = text("template");
        ml_template.required = false;
        let mut ml_status = text("status");
        ml_status.system = true;
        let mut ml_error = text("error");
        ml_error.required = false;
        let mut ml_message_id = text("messageId");
        ml_message_id.required = false;
        let pos = mail_log.fields.len() - 2;
        mail_log.fields.splice(
            pos..pos,
            [
                ml_to,
                ml_subject,
                ml_template,
                ml_status,
                ml_error,
                ml_message_id,
            ],
        );
        mail_log.indexes = vec![
            "CREATE INDEX `idx_mailLog_created` ON `_mailLog` (created)".into(),
            "CREATE INDEX `idx_mailLog_status` ON `_mailLog` (status)".into(),
        ];

        // No-code email triggers (`crate::email_triggers` in the server
        // crate): fire `template` through the ordinary send pipeline
        // whenever `event` happens on `collection`, entirely from the
        // dashboard/Records API — the same "no Rust code, no redeploy"
        // shape as `_webhooks`, and superuser-only end to end for the
        // same reason (an admin-configured integration point, not
        // something a non-superuser record should read or edit).
        let mut email_triggers = Collection::new("_emailTriggers", CollectionType::Base);
        email_triggers.system = true;
        let mut et2_collection = text("collection");
        et2_collection.required = true;
        let mut et2_event = Field::new(
            "event",
            FieldKind::Select {
                values: vec!["create".into(), "update".into(), "delete".into()],
                max_select: 1,
            },
        );
        et2_event.required = true;
        let mut et2_template = text("template");
        et2_template.required = true;
        let mut et2_to_field = text("toField");
        et2_to_field.required = true;
        // Same expression language as `sendRule`/every other API rule,
        // evaluated with `crates/filter`'s in-process `evaluate()`
        // against the written record's own fields (bare identifiers —
        // `status = "paid"` — not a `@request.*`/`@record.*` macro,
        // which this condition has no need of). `null` here means
        // "always fire" (there is no "who may configure this" tension
        // the way `sendRule`'s `null` guards, so it stays a plain
        // optional `Text` field rather than `sendRule`'s `Json` one).
        let mut et2_condition = text("condition");
        et2_condition.required = false;
        // Not `required`: a required `bool`'s zero value (`false`) reads
        // as blank (see `cratebase_db::validate::is_blank`/`required`'s
        // own doc comment — the same "0 counts as blank" footgun as a
        // required number), which would make a trigger impossible to
        // *disable* through the ordinary update API. Leaving it optional
        // also makes "not yet set" default to `false` (disabled) — the
        // safer reading for a freshly created trigger.
        let et2_enabled = Field::new("enabled", FieldKind::Bool {});
        // Extra literal values merged onto the default `{ record }`
        // template data — see `crate::email_triggers`'s module doc for
        // the exact merge order.
        let mut et2_data_map = Field::new("dataMap", FieldKind::Json { max_size: 0 });
        et2_data_map.required = false;
        let pos = email_triggers.fields.len() - 2;
        email_triggers.fields.splice(
            pos..pos,
            [
                et2_collection,
                et2_event,
                et2_template,
                et2_to_field,
                et2_condition,
                et2_enabled,
                et2_data_map,
            ],
        );
        email_triggers.indexes = vec![
            "CREATE INDEX `idx_emailTriggers_collection` ON `_emailTriggers` (collection)".into(),
        ];

        // Backing store for images uploaded through the dashboard's
        // email-template visual editor (`@react-email/editor`'s
        // image-upload plugin) — created via the ordinary generic
        // records multipart-create path
        // (`crate::routes::records`/`POST /api/collections/_emailAssets/
        // records`), which the default `None` create/update/delete
        // rules below already restrict to a superuser or API key, same
        // as every other system collection here. `file` is deliberately
        // *not* `protected`: `crates/server/src/routes/files.rs`
        // downloads an unprotected file regardless of the owning
        // collection's `viewRule`, which is exactly what an emailed
        // `<img src>` needs — the recipient's mail client has no
        // superuser session to present.
        let mut email_assets = Collection::new("_emailAssets", CollectionType::Base);
        email_assets.system = true;
        let mut ea_file = Field::new(
            "file",
            FieldKind::File {
                max_select: 1,
                max_size: 8 * 1024 * 1024,
                mime_types: vec![
                    "image/jpeg".into(),
                    "image/png".into(),
                    "image/gif".into(),
                    "image/webp".into(),
                    "image/svg+xml".into(),
                ],
                thumbs: vec![],
                protected: false,
            },
        );
        ea_file.required = true;
        let pos = email_assets.fields.len() - 2;
        email_assets.fields.insert(pos, ea_file);

        // In-app notifications (`crate::notify` in the server crate; see
        // `POST /api/notifications/send`, `$notify.send`,
        // `GET /api/notifications/unread-count`,
        // `POST /api/notifications/read-all`). `recipient` is stored as
        // the same `collectionRef`/`recordRef` pair as
        // `_push_subscriptions`/`_sessions`/... above, since a
        // notification can go to a record in *any* auth collection, not
        // just `users` — `owner_rule` (already `recordRef =
        // @request.auth.id && collectionRef = @request.auth.collectionId`)
        // is exactly "this is my own notification" for that shape. Only
        // the recipient may list/view/update/delete their own rows; the
        // *only* field they may ever change through the generic records
        // API is `readAt` — a rule can filter which rows are visible, but
        // not which fields of an allowed row may change, so that
        // restriction is enforced in
        // `crates/server/src/routes/records.rs`'s `update_record`
        // instead (see `Collection::is_notifications`). `type`/`title`/
        // `body` describe the notification; `data` is arbitrary JSON the
        // client can act on; `link` is an optional deep link; `readAt` is
        // `null` until the recipient marks it read. Creation stays
        // superuser/API-key only (`create_rule` stays `None`): the only
        // intended writer is the server's own `$notify.send`/
        // `POST /api/notifications/send` pipeline, never a rule-driven
        // client path.
        let mut notifications = Collection::new("_notifications", CollectionType::Base);
        notifications.system = true;
        notifications.list_rule = owner_rule.clone();
        notifications.view_rule = owner_rule.clone();
        notifications.update_rule = owner_rule.clone();
        notifications.delete_rule = owner_rule.clone();
        let mut n_data = Field::new("data", FieldKind::Json { max_size: 0 });
        n_data.required = false;
        let mut n_link = Field::new(
            "link",
            FieldKind::Url {
                except_domains: vec![],
                only_domains: vec![],
            },
        );
        n_link.required = false;
        let mut n_read_at = Field::new(
            "readAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        n_read_at.required = false;
        let pos = notifications.fields.len() - 2;
        notifications.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("recordRef"),
                text("type"),
                text("title"),
                text("body"),
                n_data,
                n_link,
                n_read_at,
            ],
        );
        // One composite index covers both `GET /api/notifications/unread-count`
        // (`WHERE collectionRef = ? AND recordRef = ? AND readAt IS NULL`,
        // a prefix of this index) and paginated list/retention queries
        // that also sort or filter on `created` — see the task brief's
        // "unread counts and pagination are index-backed" requirement.
        notifications.indexes = vec![
            "CREATE INDEX `idx_notifications_recipient` ON `_notifications` (collectionRef, recordRef, readAt, created)".into(),
        ];

        // Realtime channel configuration (`crate::realtime`'s channel/
        // presence support, `crate::routes::realtime_channels`). A row's
        // `name` is either an exact channel name or a prefix pattern
        // ending in `*` (`"room:*"` matches `"room:42"`, with the
        // matched suffix exposed to `subscribeRule`/`publishRule` as
        // `@request.data.suffix`; the full channel name is always
        // `@request.data.channel`). No matching row at all means the
        // channel is disabled — the secure default — never "public": an
        // operator must explicitly configure a channel (or a covering
        // prefix) before any client may subscribe to or publish on it.
        // `subscribeRule`/`publishRule` are `Json`-kind fields for the
        // same reason `_emailTemplates.sendRule` is (see its own comment
        // above): `null` means superuser/API-key only, `""` means
        // anyone, and anything else is a filter-rule expression
        // evaluated per caller. Superuser-only CRUD end to end (an
        // operator-configured integration point, the same trust tier as
        // `_webhooks`/`_emailTriggers`).
        let mut channels = Collection::new("_channels", CollectionType::Base);
        channels.system = true;
        let mut ch_subscribe_rule = Field::new("subscribeRule", FieldKind::Json { max_size: 0 });
        ch_subscribe_rule.required = false;
        let mut ch_publish_rule = Field::new("publishRule", FieldKind::Json { max_size: 0 });
        ch_publish_rule.required = false;
        let pos = channels.fields.len() - 2;
        channels
            .fields
            .splice(pos..pos, [text("name"), ch_subscribe_rule, ch_publish_rule]);
        channels.indexes =
            vec!["CREATE UNIQUE INDEX `idx_channels_name` ON `_channels` (name)".into()];

        // Presigned direct-upload claim tickets (`POST /api/files/presign`
        // in the server crate): one row per outstanding upload, superuser-
        // only end to end like `_sessions`/`_bans` above — a client never
        // reads this collection directly, only through the presign
        // endpoint (which returns the raw token once, never stored) and
        // the ordinary record create/update path (which consumes a token
        // it's handed in a file field's value). `tokenHash` is
        // `sha256(token)`, same convention as `_sessions`/`_magicLinks`.
        // `recordRef` is blank for a presign ahead of a *create* (the
        // record doesn't exist yet); `status` moves from `"pending"` to
        // `"consumed"` the moment a create/update call claims it, so a
        // reused token is rejected rather than silently attaching the
        // same upload twice. `expiresAt` rows past due are removed by the
        // storage cleanup cron (`crate::routes::files`, server crate).
        let mut pending_uploads = Collection::new("_pendingUploads", CollectionType::Base);
        pending_uploads.system = true;
        let mut pu_record_ref = text("recordRef");
        pu_record_ref.required = false;
        let mut pu_token_hash = text("tokenHash");
        pu_token_hash.hidden = true;
        let mut pu_size = Field::new("size", FieldKind::default_for(FieldType::Number));
        pu_size.system = true;
        let mut pu_status = text("status");
        pu_status.system = true;
        let mut pu_expires_at = Field::new(
            "expiresAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        pu_expires_at.system = true;
        pu_expires_at.required = true;
        let pos = pending_uploads.fields.len() - 2;
        pending_uploads.fields.splice(
            pos..pos,
            [
                text("collectionRef"),
                text("field"),
                pu_record_ref,
                text("filename"),
                text("key"),
                pu_size,
                text("mime"),
                pu_token_hash,
                pu_status,
                pu_expires_at,
            ],
        );
        pending_uploads.indexes = vec![
            "CREATE UNIQUE INDEX `idx_pendingUploads_tokenHash` ON `_pendingUploads` (tokenHash)"
                .into(),
            "CREATE INDEX `idx_pendingUploads_expiresAt` ON `_pendingUploads` (expiresAt)".into(),
        ];

        // One row per delivery *attempt series* for one `_webhooks` row on
        // one record event — `crate::webhook_deliveries` (server crate)
        // owns the whole lifecycle: `dispatch_after_success` in
        // `crate::webhooks` inserts one `pending` row instead of delivering
        // inline, a dedicated always-on ticker (independent of
        // `settings.queue.enabled`; see that module's doc for why) claims
        // due rows and POSTs, and each outcome is written back onto the
        // same row (`attempts`/`status`/`responseCode`/`responseBody`/
        // `durationMs`/`nextAttemptAt`/`error`) rather than through
        // `records::update`, same reasoning as `_webhooks`'s own status
        // write-back. Superuser-only end to end, same trust tier as
        // `_webhooks` itself: a delivery's `payload`/`responseBody` can
        // carry another collection's data verbatim.
        let mut webhook_deliveries = Collection::new("_webhookDeliveries", CollectionType::Base);
        webhook_deliveries.system = true;
        let mut wd_payload = Field::new("payload", FieldKind::Json { max_size: 0 });
        wd_payload.system = true;
        let mut wd_attempts = Field::new(
            "attempts",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        wd_attempts.system = true;
        let mut wd_max_attempts = Field::new(
            "maxAttempts",
            FieldKind::Number {
                min: Some(1.0),
                max: None,
                only_int: true,
            },
        );
        wd_max_attempts.system = true;
        let mut wd_status = text("status");
        wd_status.system = true;
        let mut wd_response_code = text("responseCode");
        wd_response_code.required = false;
        let mut wd_response_body = text("responseBody");
        wd_response_body.required = false;
        let mut wd_duration_ms = Field::new(
            "durationMs",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        wd_duration_ms.required = false;
        let mut wd_next_attempt_at = Field::new(
            "nextAttemptAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        wd_next_attempt_at.system = true;
        let mut wd_error = text("error");
        wd_error.required = false;
        let mut wd_delivered_at = Field::new(
            "deliveredAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        wd_delivered_at.required = false;
        let pos = webhook_deliveries.fields.len() - 2;
        webhook_deliveries.fields.splice(
            pos..pos,
            [
                text("webhookRef"),
                text("event"),
                text("collectionRef"),
                text("recordId"),
                wd_payload,
                wd_attempts,
                wd_max_attempts,
                wd_status,
                wd_response_code,
                wd_response_body,
                wd_duration_ms,
                wd_next_attempt_at,
                wd_error,
                wd_delivered_at,
            ],
        );
        webhook_deliveries.indexes = vec![
            "CREATE INDEX `idx_webhookDeliveries_status_nextAttemptAt` ON `_webhookDeliveries` (status, nextAttemptAt)".into(),
            "CREATE INDEX `idx_webhookDeliveries_webhookRef` ON `_webhookDeliveries` (webhookRef)".into(),
        ];

        // One row per cron *run* — both PocketBase-style `_cron_jobs` SQL
        // jobs and JS `cronAdd` jobs (which have no row of their own
        // anywhere else) — so the dashboard can show a run history next to
        // each, not just the single "last run" columns `_cron_jobs` itself
        // carries. See `crate::cron_history` (server crate) for the writer
        // and the multi-node advisory-lock guard that keeps a Postgres
        // cluster from logging (and running) the same tick on every node.
        let mut cron_runs = Collection::new("_cronRuns", CollectionType::Base);
        cron_runs.system = true;
        let mut cr_source = text("source");
        cr_source.system = true;
        let mut cr_status = text("status");
        cr_status.system = true;
        let mut cr_started_at = Field::new(
            "startedAt",
            FieldKind::Date {
                min: None,
                max: None,
            },
        );
        cr_started_at.system = true;
        let mut cr_duration_ms = Field::new(
            "durationMs",
            FieldKind::Number {
                min: Some(0.0),
                max: None,
                only_int: true,
            },
        );
        cr_duration_ms.required = false;
        let mut cr_message = text("message");
        cr_message.required = false;
        let pos = cron_runs.fields.len() - 2;
        cron_runs.fields.splice(
            pos..pos,
            [
                text("jobId"),
                cr_source,
                cr_status,
                cr_started_at,
                cr_duration_ms,
                cr_message,
            ],
        );
        cron_runs.indexes = vec![
            "CREATE INDEX `idx_cronRuns_jobId_startedAt` ON `_cronRuns` (jobId, startedAt)".into(),
        ];

        vec![
            external,
            mfas,
            otps,
            totps,
            magic_links,
            origins,
            sessions,
            bans,
            cron_jobs,
            rpc,
            webhooks,
            teams,
            team_members,
            llm_usage,
            api_keys,
            push_subscriptions,
            audit_log,
            email_templates,
            mail_log,
            email_triggers,
            email_assets,
            notifications,
            channels,
            pending_uploads,
            webhook_deliveries,
            cron_runs,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_language_defaults_to_simple_and_round_trips() {
        let mut c = Collection::new("posts", CollectionType::Base);
        assert_eq!(c.search_language_or_default(), "simple");
        assert!(!c.has_search_index());

        c.fields.push(Field::new(
            "title",
            FieldKind::Text {
                min: 0,
                max: 0,
                pattern: String::new(),
                autogenerate_pattern: String::new(),
                primary_key: false,
            },
        ));
        c.fields.last_mut().unwrap().searchable = true;
        c.search_language = Some("english".into());
        assert!(c.has_search_index());
        assert_eq!(c.searchable_fields().count(), 1);
        assert_eq!(c.search_language_or_default(), "english");

        let v = c.to_json();
        assert_eq!(v["searchLanguage"], "english");
        let back: Collection = serde_json::from_value(v).unwrap();
        assert_eq!(back.search_language.as_deref(), Some("english"));
    }

    #[test]
    fn owner_field_defaults_to_none_and_round_trips() {
        let mut c = Collection::new("photos", CollectionType::Base);
        assert!(c.owner_field.is_none());
        c.owner_field = Some("owner".into());
        let v = c.to_json();
        assert_eq!(v["ownerField"], "owner");
        let back: Collection = serde_json::from_value(v).unwrap();
        assert_eq!(back.owner_field.as_deref(), Some("owner"));

        let v2 = Collection::new("posts", CollectionType::Base).to_json();
        assert!(v2["ownerField"].is_null());
    }

    #[test]
    fn base_collection_json_has_no_auth_or_view_blocks() {
        let c = Collection::new("posts", CollectionType::Base);
        let v = c.to_json();
        assert_eq!(v["type"], "base");
        assert!(v.get("viewQuery").is_none());
        assert!(v.get("passwordAuth").is_none());
        assert_eq!(v["fields"][0]["name"], "id");
        assert_eq!(v["id"], "pbc_1125843985");
    }

    #[test]
    fn auth_collection_json_matches_pocketbase_defaults() {
        let c = Collection::default_users();
        let v = c.to_json();
        assert_eq!(v["id"], "_pb_users_auth_");
        assert_eq!(v["passwordAuth"]["identityFields"][0], "email");
        assert_eq!(v["authToken"]["duration"], 432000);
        assert_eq!(v["otp"]["length"], 8);
        assert_eq!(v["mfa"]["duration"], 600);
        assert_eq!(v["authRule"], "");
        assert!(v["manageRule"].is_null());
        let names: Vec<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "id",
                "password",
                "tokenKey",
                "email",
                "emailVisibility",
                "verified",
                "name",
                "avatar",
                "created",
                "updated",
            ]
        );
        assert_eq!(Collection::default_superusers().id, "pbc_3142635823");
        assert_eq!(c.fields[2].id, "text2504183744");
        assert_eq!(c.fields[3].id, "email3885137012");
    }

    #[test]
    fn default_superusers_has_a_required_owner_admin_role_field() {
        let c = Collection::default_superusers();
        let names: Vec<&str> = c.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "id",
                "password",
                "tokenKey",
                "email",
                "emailVisibility",
                "verified",
                "role",
                "created",
                "updated",
            ]
        );
        let role = c.fields.iter().find(|f| f.name == "role").unwrap();
        assert!(role.required);
        match &role.kind {
            FieldKind::Select { values, max_select } => {
                assert_eq!(values, &vec!["owner".to_string(), "admin".to_string()]);
                assert_eq!(*max_select, 1);
            }
            other => panic!("expected a select field, got {other:?}"),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let c = Collection::default_users();
        let v = c.to_json();
        let back: Collection = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(back.name, "users");
        assert!(back.is_auth());
        assert_eq!(back.auth, c.auth);
        assert_eq!(back.to_json(), v);
    }

    #[test]
    fn round_trips_a_vector_field_through_json() {
        let mut c = Collection::new("chunks", CollectionType::Base);
        let pos = c.fields.len() - 2;
        c.fields.insert(
            pos,
            Field::new(
                "embedding",
                FieldKind::Vector {
                    dimensions: 4,
                    embedding: Some(crate::field::EmbeddingConfig {
                        provider: "echo".into(),
                        model: String::new(),
                        source_field: "body".into(),
                    }),
                },
            ),
        );
        let v = c.to_json();
        assert_eq!(v["fields"][pos]["type"], "vector");
        assert_eq!(v["fields"][pos]["dimensions"], 4);
        assert_eq!(v["fields"][pos]["embedding"]["sourceField"], "body");
        let back: Collection = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(back.to_json(), v);
        match &back.fields[pos].kind {
            FieldKind::Vector {
                dimensions,
                embedding: Some(cfg),
            } => {
                assert_eq!(*dimensions, 4);
                assert_eq!(cfg.source_field, "body");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lenient_input_for_new_base_collection() {
        let mut c: Collection =
            serde_json::from_str(r#"{"name":"posts","type":"base","fields":[{"name":"title","type":"text","required":true}]}"#)
                .unwrap();
        assert!(c.id.is_empty());
        c.ensure_system_fields();
        assert_eq!(c.fields[0].name, "id");
        assert_eq!(c.fields[1].name, "title");
        assert_eq!(c.fields[1].id, "text724990059");
    }

    #[test]
    fn system_collections_match_fixture_ids() {
        let sys = Collection::default_system_collections();
        let ids: Vec<&str> = sys.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                crate::ids::collection_id("base", "_externalAuths").as_str(),
                "pbc_2279338944",
                "pbc_1638494021",
                crate::ids::collection_id("base", "_totps").as_str(),
                crate::ids::collection_id("base", "_magicLinks").as_str(),
                crate::ids::collection_id("base", "_authOrigins").as_str(),
                crate::ids::collection_id("base", "_sessions").as_str(),
                crate::ids::collection_id("base", "_bans").as_str(),
                crate::ids::collection_id("base", "_cron_jobs").as_str(),
                crate::ids::collection_id("base", "_rpc").as_str(),
                crate::ids::collection_id("base", "_webhooks").as_str(),
                crate::ids::collection_id("base", "_teams").as_str(),
                crate::ids::collection_id("base", "_team_members").as_str(),
                crate::ids::collection_id("base", "_llm_usage").as_str(),
                crate::ids::collection_id("base", "_api_keys").as_str(),
                crate::ids::collection_id("base", "_push_subscriptions").as_str(),
                crate::ids::collection_id("base", "_audit_log").as_str(),
                crate::ids::collection_id("base", "_emailTemplates").as_str(),
                crate::ids::collection_id("base", "_mailLog").as_str(),
                crate::ids::collection_id("base", "_emailTriggers").as_str(),
                crate::ids::collection_id("base", "_emailAssets").as_str(),
                crate::ids::collection_id("base", "_notifications").as_str(),
                crate::ids::collection_id("base", "_channels").as_str(),
                crate::ids::collection_id("base", "_pendingUploads").as_str(),
                crate::ids::collection_id("base", "_webhookDeliveries").as_str(),
                crate::ids::collection_id("base", "_cronRuns").as_str(),
            ]
        );
        assert_eq!(Collection::default_superusers().id, "pbc_3142635823");
    }
}
