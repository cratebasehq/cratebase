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
    /// `_cron_jobs`, `_webhooks`, `_teams`, `_team_members` base
    /// collections, with PocketBase's rules and indexes (`_cron_jobs`,
    /// `_webhooks`, `_teams` and `_team_members` have no PocketBase
    /// equivalent — see their own comments).
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
        push_subscriptions.delete_rule = owner_rule;
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

        vec![
            external,
            mfas,
            otps,
            origins,
            cron_jobs,
            webhooks,
            teams,
            team_members,
            llm_usage,
            api_keys,
            push_subscriptions,
            audit_log,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                crate::ids::collection_id("base", "_authOrigins").as_str(),
                crate::ids::collection_id("base", "_cron_jobs").as_str(),
                crate::ids::collection_id("base", "_webhooks").as_str(),
                crate::ids::collection_id("base", "_teams").as_str(),
                crate::ids::collection_id("base", "_team_members").as_str(),
                crate::ids::collection_id("base", "_llm_usage").as_str(),
                crate::ids::collection_id("base", "_api_keys").as_str(),
                crate::ids::collection_id("base", "_push_subscriptions").as_str(),
                crate::ids::collection_id("base", "_audit_log").as_str(),
            ]
        );
        assert_eq!(Collection::default_superusers().id, "pbc_3142635823");
    }
}
