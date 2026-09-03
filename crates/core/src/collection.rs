use serde::{Deserialize, Serialize};

use crate::field::Field;

/// The kind of a collection. `Auth` collections get built-in password
/// authentication endpoints and reserved fields (email, password, etc).
/// `View` collections are read-only, backed by a custom SQL query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionType {
    Base,
    Auth,
    View,
}

/// Options specific to `Auth`-typed collections.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthOptions {
    pub min_password_length: Option<u32>,
    /// The schema field that identifies an auth record for login, e.g.
    /// `"email"` (default) or `"username"`. Cratebase auto-manages this
    /// column (uniqueness, presence) the same way it always managed a
    /// hardcoded `email` column; it just isn't hardcoded to that name
    /// anymore.
    pub identity_field: Option<String>,
    pub require_email_verification: Option<bool>,
    pub token_ttl_seconds: Option<i64>,
    /// When `true`, a successful password login (`auth-with-password`)
    /// doesn't issue a session token directly — it emails a one-time code
    /// (the same mechanism as `request-otp`/`auth-with-otp`) and returns a
    /// short-lived pending marker instead, which `/mfa/confirm` exchanges
    /// for the real session token once the code is presented back.
    pub mfa_required: Option<bool>,
}

impl AuthOptions {
    /// The configured identity field name, defaulting to `"email"` for
    /// collections created before this option existed.
    pub fn identity_field(&self) -> &str {
        self.identity_field.as_deref().unwrap_or("email")
    }

    /// Whether the identity field should be validated/rendered as an email
    /// address. Any other identity field name (e.g. `"username"`) is
    /// treated as free-form text.
    pub fn identity_is_email(&self) -> bool {
        self.identity_field() == "email"
    }

    /// Whether password logins for this collection require a second-factor
    /// OTP confirmation before a session token is issued.
    pub fn mfa_required(&self) -> bool {
        self.mfa_required.unwrap_or(false)
    }
}

/// A dynamic, user-defined data collection. Persisted in the system
/// `_collections` table and materialized as a real SQL table
/// (`cb_<name>`) by `cratebase-db`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub collection_type: CollectionType,
    #[serde(default)]
    pub schema: Vec<Field>,
    /// SQL-like filter expression required to list records. `None` = only
    /// authenticated superusers may access. `Some("")` = public.
    #[serde(default)]
    pub list_rule: Option<String>,
    #[serde(default)]
    pub view_rule: Option<String>,
    #[serde(default)]
    pub create_rule: Option<String>,
    #[serde(default)]
    pub update_rule: Option<String>,
    #[serde(default)]
    pub delete_rule: Option<String>,
    #[serde(default)]
    pub auth_options: AuthOptions,
    /// Raw SELECT statement backing a `View` collection.
    #[serde(default)]
    pub view_query: Option<String>,
    pub created: String,
    pub updated: String,
}

impl Collection {
    pub fn table_name(&self) -> String {
        format!("cb_{}", self.name)
    }

    pub fn is_auth(&self) -> bool {
        matches!(self.collection_type, CollectionType::Auth)
    }

    pub fn is_view(&self) -> bool {
        matches!(self.collection_type, CollectionType::View)
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.schema.iter().find(|f| f.name == name)
    }
}
