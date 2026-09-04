//! The event payloads carried down the hook chains declared in
//! [`crate::hooks::Hooks`].
//!
//! Payloads are deliberately thin: the *shape* is what matters, so the
//! remaining services (realtime, batch) and the JS runtime can add fields
//! without renaming a hook or changing a registration signature. Every struct owns a `hook_chain` cursor and,
//! where PocketBase supports tagged hooks, a `tags` list holding the
//! collection's name and id.

use std::sync::Arc;

use cratebase_core::{Collection, Record, Settings};
use cratebase_mailer::Message;

use crate::app::{App, TxApp};
use crate::extract::{Auth, RequestInfo};
use crate::hooks::Chain;
use crate::impl_event;

/// Both identifiers PocketBase accepts in a hook tag: the collection's
/// name and its id.
pub fn collection_tags(collection: &Collection) -> Vec<String> {
    vec![collection.name.clone(), collection.id.clone()]
}

macro_rules! event_struct {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty ),* $(,)? }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $( $(#[$fmeta])* pub $field : $ty, )*
            pub(crate) hook_chain: Chain<Self>,
        }
        impl_event!($name);
        impl $name {
            #[allow(clippy::too_many_arguments)]
            pub fn new( $( $field : $ty ),* ) -> Self {
                Self { $( $field ),*, hook_chain: Chain::default() }
            }
        }
    };
    (
        $(#[$meta:meta])*
        $name:ident tagged { $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty ),* $(,)? }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $( $(#[$fmeta])* pub $field : $ty, )*
            /// Collection name and id; tagged handlers filter on these.
            pub tags: Vec<String>,
            pub(crate) hook_chain: Chain<Self>,
        }
        impl_event!($name, tags);
        impl $name {
            #[allow(clippy::too_many_arguments)]
            pub fn new( $( $field : $ty, )* tags: Vec<String> ) -> Self {
                Self { $( $field, )* tags, hook_chain: Chain::default() }
            }
        }
    };
}

// ---------------------------------------------------------------- lifecycle

event_struct! {
    /// Fired once inside [`App::bootstrap`]. A handler may replace the
    /// whole bootstrap by not calling `next()`.
    BootstrapEvent { pub app: App }
}

event_struct! {
    /// Fired after the router is assembled and before the listener binds.
    /// Plugins mount their routes by mutating `router`.
    ServeEvent {
        pub app: App,
        pub router: axum::Router,
        pub address: String,
    }
}

event_struct! {
    /// Fired on shutdown. `is_restart` is true when the process is about
    /// to re-exec itself (a backup restore).
    TerminateEvent { pub app: App, pub is_restart: bool }
}

// ------------------------------------------------------------------ records

event_struct! {
    /// `onRecordEnrich`: last chance to add/remove fields before a record
    /// is serialised into a response.
    RecordEnrichEvent tagged {
        pub app: App,
        pub collection: Arc<Collection>,
        pub record: Record,
        pub auth: Option<Auth>,
    }
}

event_struct! {
    /// The write-path events (`onRecordValidate`, `onRecordCreate` and
    /// its `*Execute` sibling, and the same for update/delete). `app` is
    /// the transactional handle, so a handler's own DB calls join the
    /// caller's transaction.
    RecordEvent tagged {
        pub app: TxApp,
        pub collection: Arc<Collection>,
        pub record: Record,
        /// The stored row before the change (update/delete only).
        pub previous: Option<Record>,
    }
}

event_struct! {
    /// `onRecordAfterCreateError` and friends: fired after the
    /// transaction rolled back.
    RecordErrorEvent tagged {
        pub app: App,
        pub collection: Arc<Collection>,
        pub record: Record,
        pub error: String,
    }
}

event_struct! {
    /// `onRecordsListRequest` / `onRecordViewRequest` /
    /// `onRecord{Create,Update,Delete}Request` and every auth request
    /// variant: the HTTP-facing wrapper around the write path.
    ///
    /// `record` carries the record the request resolved to (the created
    /// or updated row, the authenticated record, ...); it is `None` for a
    /// list, whose page is not a single record.
    RecordRequestEvent tagged {
        pub app: App,
        pub collection: Arc<Collection>,
        pub request: RequestInfo,
        pub auth: Option<Auth>,
        pub record: Option<Record>,
    }
}

// -------------------------------------------------------------- collections

event_struct! {
    /// `onCollection{Create,Update,Delete}` and their `*Execute` forms.
    CollectionEvent tagged {
        pub app: TxApp,
        pub collection: Collection,
        pub previous: Option<Collection>,
    }
}

event_struct! {
    /// `onCollections{List,View,Create,Update,Delete,Import}Request`.
    CollectionRequestEvent tagged {
        pub app: App,
        pub request: RequestInfo,
        pub auth: Option<Auth>,
        pub collection: Option<Collection>,
    }
}

// ------------------------------------------------------------------- mailer

event_struct! {
    /// `onMailerSend`: every outgoing message passes through here, so a
    /// hook can rewrite it or swap the transport by not calling `next()`.
    MailerEvent { pub app: App, pub message: Message }
}

event_struct! {
    /// The five record-scoped sends (verification, password reset, email
    /// change, OTP, auth alert).
    MailerRecordEvent tagged {
        pub app: App,
        pub collection: Arc<Collection>,
        pub record: Record,
        pub message: Message,
        /// Template variables already substituted into `message`.
        pub meta: serde_json::Value,
    }
}

// ----------------------------------------------------------------- realtime

event_struct! {
    /// `onRealtimeConnectRequest`: an SSE client just connected.
    RealtimeConnectEvent {
        pub app: App,
        pub client_id: String,
        pub auth: Option<Auth>,
        pub idle_timeout_secs: i64,
    }
}

event_struct! {
    /// `onRealtimeSubscribeRequest`: the client sent its topic list.
    RealtimeSubscribeEvent {
        pub app: App,
        pub client_id: String,
        pub auth: Option<Auth>,
        pub subscriptions: Vec<String>,
    }
}

event_struct! {
    /// `onRealtimeMessageSend`: one SSE frame about to be written.
    RealtimeMessageEvent {
        pub app: App,
        pub client_id: String,
        pub name: String,
        pub data: serde_json::Value,
    }
}

// -------------------------------------------------------------------- files

event_struct! {
    /// `onFileDownloadRequest`.
    FileDownloadEvent tagged {
        pub app: App,
        pub collection: Arc<Collection>,
        pub record: Option<Record>,
        /// Storage key being served.
        pub key: String,
        /// Filename offered to the client.
        pub served_name: String,
    }
}

event_struct! {
    /// `onFileTokenRequest`: a short-lived token for a protected file.
    FileTokenEvent {
        pub app: App,
        pub auth: Option<Auth>,
        pub token: String,
    }
}

// ------------------------------------------------------------------ backups

event_struct! {
    /// `onBackupCreate`: `name` is the ZIP key inside the backups store.
    BackupCreateEvent { pub app: App, pub name: String }
}

event_struct! {
    /// `onBackupRestore`: fired before the data directory is swapped and
    /// the process re-execs.
    BackupRestoreEvent { pub app: App, pub name: String }
}

// ----------------------------------------------------------------- settings

event_struct! {
    /// `onSettingsListRequest`.
    SettingsListEvent {
        pub app: App,
        pub request: RequestInfo,
        pub settings: Arc<Settings>,
    }
}

event_struct! {
    /// `onSettingsUpdateRequest`: `new_settings` is the merged result and
    /// is what gets persisted, so a handler can still amend it.
    SettingsUpdateEvent {
        pub app: App,
        pub request: RequestInfo,
        pub old_settings: Arc<Settings>,
        pub new_settings: Settings,
    }
}

event_struct! {
    /// `onSettingsReload`: fired after settings were swapped in memory
    /// (also on boot), so derived services can be rebuilt.
    SettingsReloadEvent { pub app: App, pub settings: Arc<Settings> }
}

// -------------------------------------------------------------------- batch

event_struct! {
    /// `onBatchRequest`.
    BatchRequestEvent {
        pub app: App,
        pub request: RequestInfo,
        pub auth: Option<Auth>,
        /// One entry per sub-request, as parsed from the payload.
        pub requests: Vec<serde_json::Value>,
    }
}
