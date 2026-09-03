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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

impl Default for EmailTemplate {
    fn default() -> Self {
        EmailTemplate {
            subject: String::new(),
            body: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenConfig {
    pub duration: i64,
    /// Optional per-collection signing secret suffix; empty means the
    /// app secret alone (plus the record's `tokenKey`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
}

impl Default for TokenConfig {
    fn default() -> Self {
        TokenConfig {
            duration: 0,
            secret: String::new(),
        }
    }
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
    /// Write-only: never serialized back to clients.
    #[serde(skip_serializing)]
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
            format!("CREATE UNIQUE INDEX `idx_tokenKey_{}` ON `users` (`tokenKey`)", c.id),
            format!(
                "CREATE UNIQUE INDEX `idx_email_{}` ON `users` (`email`) WHERE `email` != ''",
                c.id
            ),
        ];
        c
    }

    /// The built-in `_superusers` auth collection.
    pub fn default_superusers() -> Self {
        let mut c = Collection::new(crate::SUPERUSERS_COLLECTION, CollectionType::Auth);
        c.system = true;
        c.auth.manage_rule = None;
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

    /// The system `_externalAuths`, `_mfas`, `_otps`, `_authOrigins`
    /// base collections, with PocketBase's rules and indexes.
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
            "CREATE INDEX `idx_mfas_collectionRef_recordRef` ON `_mfas` (collectionRef,recordRef)".into(),
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
            "CREATE INDEX `idx_otps_collectionRef_recordRef` ON `_otps` (collectionRef, recordRef)".into(),
        ];

        let mut origins = Collection::new("_authOrigins", CollectionType::Base);
        origins.system = true;
        origins.list_rule = owner_rule.clone();
        origins.view_rule = owner_rule.clone();
        origins.delete_rule = owner_rule;
        let pos = origins.fields.len() - 2;
        origins.fields.splice(
            pos..pos,
            [text("collectionRef"), text("recordRef"), text("fingerprint")],
        );
        origins.indexes = vec![
            "CREATE UNIQUE INDEX `idx_authOrigins_unique_pairs` ON `_authOrigins` (collectionRef, recordRef, fingerprint)".into(),
        ];

        vec![external, mfas, otps, origins]
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
                "updated"
            ]
        );
        assert_eq!(c.fields[1].id, "password901924565");
        assert_eq!(c.fields[2].id, "text2504183744");
        assert_eq!(c.fields[3].id, "email3885137012");
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
                crate::ids::collection_id("base", "_authOrigins").as_str()
            ]
        );
        assert_eq!(
            Collection::default_superusers().id,
            "pbc_3142635823"
        );
    }
}
