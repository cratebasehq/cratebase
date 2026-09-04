//! The event/hook system, with PocketBase's `e.next()` semantics.
//!
//! # Why a chain rather than a callback list
//!
//! PocketBase hooks are *middleware*, not observers. Every handler
//! receives the event and decides whether the rest of the chain — and,
//! last of all, the framework's own action (the *finalizer*) — runs at
//! all:
//!
//! ```ignore
//! app.hooks().on_record_create.bind_func(|e| Box::pin(async move {
//!     if forbidden(e) { return Err(AppError::forbidden("")); } // aborts
//!     e.next().await?;                                         // creates
//!     audit(e).await;                                          // after
//!     Ok(())
//! }));
//! ```
//!
//! A handler that returns without calling `next()` stops the chain and
//! the framework action never happens (PocketBase uses exactly this to
//! let a hook replace a built-in behaviour). An error aborts the chain
//! and propagates to the caller.
//!
//! # Why the cursor lives on the event
//!
//! `next()` has to re-enter the chain from inside a handler that already
//! holds `&mut E`. Threading a separate "chain" argument through every
//! handler signature would make `bind_func` closures unwieldy, so the
//! cursor rides along *on the event* in a [`Chain`] field. [`trigger`]
//! takes an `Arc<[Handler]>` snapshot of the handler list before the
//! first handler runs, so a handler that registers or unbinds another
//! handler mid-flight neither deadlocks on the registry lock nor mutates
//! the sequence it is part of. The change is visible from the next
//! trigger onwards, as in PocketBase.
//!
//! [`trigger`]: Hook::trigger

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, RwLock};

use cratebase_core::AppError;
use futures::future::BoxFuture;

/// The result every handler and finalizer returns.
pub type HookResult = Result<(), AppError>;

/// A handler body. Borrows the event for the duration of its future, so
/// handlers can mutate the payload and call `e.next()`.
pub type HandlerFn<E> = Arc<dyn for<'a> Fn(&'a mut E) -> BoxFuture<'a, HookResult> + Send + Sync>;

/// The framework's own action, run after every handler that called
/// `next()`. Consumed by the trigger.
pub type Finalizer<E> = Box<dyn for<'a> FnOnce(&'a mut E) -> BoxFuture<'a, HookResult> + Send>;

/// One registered handler.
pub struct Handler<E> {
    /// Optional identity, so [`Hook::unbind`] can remove it later. Two
    /// handlers registered with the same id replace each other, matching
    /// PocketBase's `BindFunc`/`Bind(Handler{Id})` behaviour.
    pub id: Option<String>,
    /// Lower runs first. PocketBase's default is 0.
    pub priority: i32,
    /// When non-empty the handler only fires for events carrying at least
    /// one of these tags (a collection name or id).
    pub tags: Vec<String>,
    pub func: HandlerFn<E>,
}

/// Hand-written so `E` need not be `Clone` — only the `Arc` around the
/// closure is cloned.
impl<E> Clone for Handler<E> {
    fn clone(&self) -> Self {
        Handler {
            id: self.id.clone(),
            priority: self.priority,
            tags: self.tags.clone(),
            func: self.func.clone(),
        }
    }
}

impl<E> fmt::Debug for Handler<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handler")
            .field("id", &self.id)
            .field("priority", &self.priority)
            .field("tags", &self.tags)
            .finish_non_exhaustive()
    }
}

impl<E> Handler<E> {
    pub fn new(
        func: impl for<'a> Fn(&'a mut E) -> BoxFuture<'a, HookResult> + Send + Sync + 'static,
    ) -> Self {
        Handler {
            id: None,
            priority: 0,
            tags: Vec::new(),
            func: Arc::new(func),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }
}

/// The cursor an event carries while it travels down a hook chain.
/// Default-constructed events start with an empty chain, so an event
/// triggered without any hook still works (`next()` runs the finalizer).
pub struct Chain<E> {
    handlers: Arc<[Handler<E>]>,
    index: usize,
    finalizer: Option<Finalizer<E>>,
}

impl<E> Default for Chain<E> {
    fn default() -> Self {
        Chain {
            handlers: Arc::from(Vec::new()),
            index: 0,
            finalizer: None,
        }
    }
}

impl<E> fmt::Debug for Chain<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Chain")
            .field("handlers", &self.handlers.len())
            .field("index", &self.index)
            .field("has_finalizer", &self.finalizer.is_some())
            .finish()
    }
}

/// Anything that can travel down a [`Hook`] chain.
///
/// Implementors only supply access to their [`Chain`] field (and,
/// optionally, their tags); `next()` comes for free. Use the
/// [`impl_event!`] macro rather than writing this by hand.
pub trait Event: Sized + Send {
    fn chain(&mut self) -> &mut Chain<Self>;

    /// Collection name/id (or other selector) this event belongs to.
    /// Tagged handlers only fire when they share a tag with the event.
    fn tags(&self) -> &[String] {
        &[]
    }

    /// Run the remainder of the chain: the next handler, or — once the
    /// handlers are exhausted — the framework's own action.
    ///
    /// Calling `next()` more than once from the same handler is a no-op
    /// after the finalizer has run, rather than a panic, so a buggy hook
    /// degrades instead of taking the request down.
    fn next(&mut self) -> BoxFuture<'_, HookResult> {
        Box::pin(async move {
            let (handler, finalizer) = {
                let chain = self.chain();
                let index = chain.index;
                chain.index += 1;
                match chain.handlers.get(index) {
                    Some(h) => (Some(h.func.clone()), None),
                    None => (None, chain.finalizer.take()),
                }
            };
            match (handler, finalizer) {
                (Some(func), _) => func(self).await,
                (None, Some(finalizer)) => finalizer(self).await,
                (None, None) => Ok(()),
            }
        })
    }
}

/// Implements [`Event`] for a struct holding a `hook_chain: Chain<Self>`
/// field, plus (optionally) a `tags: Vec<String>` field.
#[macro_export]
macro_rules! impl_event {
    ($ty:ty) => {
        impl $crate::hooks::Event for $ty {
            fn chain(&mut self) -> &mut $crate::hooks::Chain<Self> {
                &mut self.hook_chain
            }
        }
    };
    ($ty:ty, tags) => {
        impl $crate::hooks::Event for $ty {
            fn chain(&mut self) -> &mut $crate::hooks::Chain<Self> {
                &mut self.hook_chain
            }
            fn tags(&self) -> &[String] {
                &self.tags
            }
        }
    };
}

/// A registration point for handlers of one event type.
pub struct Hook<E> {
    /// Sorted by priority; rebuilt on every mutation so `trigger` only
    /// clones an `Arc`.
    handlers: RwLock<Arc<[Handler<E>]>>,
}

impl<E> Default for Hook<E> {
    fn default() -> Self {
        Hook {
            handlers: RwLock::new(Arc::from(Vec::new())),
        }
    }
}

impl<E> fmt::Debug for Hook<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let handlers = self.handlers.read().map(|h| h.len()).unwrap_or(0);
        f.debug_struct("Hook").field("handlers", &handlers).finish()
    }
}

impl<E: Event> Hook<E> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `handler`. A handler whose id matches an existing one
    /// replaces it in place; otherwise it is inserted in priority order,
    /// after equal-priority handlers registered before it.
    pub fn bind(&self, handler: Handler<E>) {
        let mut guard = self.handlers.write().expect("hook registry poisoned");
        let mut next: Vec<Handler<E>> = guard.iter().cloned().collect();
        if let Some(id) = &handler.id {
            next.retain(|h| h.id.as_deref() != Some(id.as_str()));
        }
        next.push(handler);
        next.sort_by_key(|h| h.priority);
        *guard = Arc::from(next);
    }

    /// Register a bare async closure with default priority and no id.
    pub fn bind_func(
        &self,
        func: impl for<'a> Fn(&'a mut E) -> BoxFuture<'a, HookResult> + Send + Sync + 'static,
    ) {
        self.bind(Handler::new(func));
    }

    /// Remove every handler registered under `id`. Returns how many went.
    pub fn unbind(&self, id: &str) -> usize {
        let mut guard = self.handlers.write().expect("hook registry poisoned");
        let before = guard.len();
        let next: Vec<Handler<E>> = guard
            .iter()
            .filter(|h| h.id.as_deref() != Some(id))
            .cloned()
            .collect();
        let removed = before - next.len();
        *guard = Arc::from(next);
        removed
    }

    /// Remove every handler.
    pub fn unbind_all(&self) {
        *self.handlers.write().expect("hook registry poisoned") = Arc::from(Vec::new());
    }

    pub fn len(&self) -> usize {
        self.handlers.read().expect("hook registry poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run the chain for `event`, ending in `finalizer` (the framework's
    /// own action). Handlers that carry tags are filtered against the
    /// event's tags before the snapshot is taken.
    pub async fn trigger(
        &self,
        event: &mut E,
        finalizer: impl for<'a> FnOnce(&'a mut E) -> BoxFuture<'a, HookResult> + Send + 'static,
    ) -> HookResult {
        let snapshot = self
            .handlers
            .read()
            .expect("hook registry poisoned")
            .clone();
        let handlers: Arc<[Handler<E>]> = if snapshot.iter().any(|h| !h.tags.is_empty()) {
            let tags: HashSet<&str> = event.tags().iter().map(String::as_str).collect();
            Arc::from(
                snapshot
                    .iter()
                    .filter(|h| {
                        h.tags.is_empty() || h.tags.iter().any(|t| tags.contains(t.as_str()))
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        } else {
            snapshot
        };

        {
            let chain = event.chain();
            chain.handlers = handlers;
            chain.index = 0;
            chain.finalizer = Some(Box::new(finalizer));
        }
        let result = event.next().await;
        // Drop any finalizer a short-circuiting handler left behind, so
        // the event can be reused (batch replays one event per request).
        event.chain().finalizer = None;
        result
    }

    /// Run the chain with nothing at the end — for pure notifications
    /// (`on_terminate`, `on_realtime_message_send`) where there is no
    /// framework action to guard.
    pub async fn trigger_bare(&self, event: &mut E) -> HookResult {
        self.trigger(event, |_| Box::pin(async { Ok(()) })).await
    }
}

// ---------------------------------------------------------------------------
// The registry of every hook the app exposes.
// ---------------------------------------------------------------------------

use crate::events::*;

macro_rules! hook_set {
    ( $( $(#[$meta:meta])* $field:ident : $ty:ty ),* $(,)? ) => {
        /// Every hook chain the app exposes, mirroring PocketBase's `App`
        /// `On*` methods one-for-one (spec §7.1). Fields are public so
        /// Rust plugins, the JS runtime and W4b's services all register
        /// the same way:
        ///
        /// ```ignore
        /// app.hooks().on_record_create.bind(
        ///     Handler::new(|e| Box::pin(async move { e.next().await }))
        ///         .with_tags(["posts"]),
        /// );
        /// ```
        #[derive(Default)]
        pub struct Hooks {
            $( $(#[$meta])* pub $field : Hook<$ty>, )*
        }

        impl Hooks {
            pub fn new() -> Self {
                Self::default()
            }

            /// Total registered handlers across every hook — used by
            /// tests and the dashboard's diagnostics.
            pub fn handler_count(&self) -> usize {
                0 $( + self.$field.len() )*
            }

            /// Remove every handler registered under `id` from every
            /// hook. The JS runtime uses this when a `pb_hooks` file is
            /// reloaded in `--dev`.
            pub fn unbind_all_by_id(&self, id: &str) -> usize {
                0 $( + self.$field.unbind(id) )*
            }
        }
    };
}

hook_set! {
    // lifecycle
    on_bootstrap: BootstrapEvent,
    on_serve: ServeEvent,
    on_terminate: TerminateEvent,

    // record read / serialisation
    on_record_enrich: RecordEnrichEvent,

    // record write path
    on_record_validate: RecordEvent,
    on_record_create: RecordEvent,
    on_record_create_execute: RecordEvent,
    on_record_after_create_success: RecordEvent,
    on_record_after_create_error: RecordErrorEvent,
    on_record_update: RecordEvent,
    on_record_update_execute: RecordEvent,
    on_record_after_update_success: RecordEvent,
    on_record_after_update_error: RecordErrorEvent,
    on_record_delete: RecordEvent,
    on_record_delete_execute: RecordEvent,
    on_record_after_delete_success: RecordEvent,
    on_record_after_delete_error: RecordErrorEvent,

    // record HTTP requests
    on_record_list_request: RecordRequestEvent,
    on_record_view_request: RecordRequestEvent,
    on_record_create_request: RecordRequestEvent,
    on_record_update_request: RecordRequestEvent,
    on_record_delete_request: RecordRequestEvent,

    // auth requests
    on_record_auth_request: RecordRequestEvent,
    on_record_auth_with_password_request: RecordRequestEvent,
    on_record_auth_with_oauth2_request: RecordRequestEvent,
    on_record_auth_with_otp_request: RecordRequestEvent,
    on_record_auth_refresh_request: RecordRequestEvent,
    on_record_request_password_reset_request: RecordRequestEvent,
    on_record_confirm_password_reset_request: RecordRequestEvent,
    on_record_request_verification_request: RecordRequestEvent,
    on_record_confirm_verification_request: RecordRequestEvent,
    on_record_request_email_change_request: RecordRequestEvent,
    on_record_confirm_email_change_request: RecordRequestEvent,
    on_record_request_otp_request: RecordRequestEvent,
    on_record_list_external_auths_request: RecordRequestEvent,
    on_record_unlink_external_auth_request: RecordRequestEvent,

    // collections
    on_collection_validate: CollectionEvent,
    on_collection_create: CollectionEvent,
    on_collection_create_execute: CollectionEvent,
    on_collection_after_create_success: CollectionEvent,
    on_collection_after_create_error: CollectionEvent,
    on_collection_update: CollectionEvent,
    on_collection_update_execute: CollectionEvent,
    on_collection_after_update_success: CollectionEvent,
    on_collection_after_update_error: CollectionEvent,
    on_collection_delete: CollectionEvent,
    on_collection_delete_execute: CollectionEvent,
    on_collection_after_delete_success: CollectionEvent,
    on_collection_after_delete_error: CollectionEvent,
    on_collections_list_request: CollectionRequestEvent,
    on_collection_view_request: CollectionRequestEvent,
    on_collection_create_request: CollectionRequestEvent,
    on_collection_update_request: CollectionRequestEvent,
    on_collection_delete_request: CollectionRequestEvent,
    on_collections_import_request: CollectionRequestEvent,

    // mailer
    on_mailer_send: MailerEvent,
    on_mailer_record_auth_alert_send: MailerRecordEvent,
    on_mailer_record_password_reset_send: MailerRecordEvent,
    on_mailer_record_verification_send: MailerRecordEvent,
    on_mailer_record_email_change_send: MailerRecordEvent,
    on_mailer_record_otp_send: MailerRecordEvent,

    // realtime
    on_realtime_connect_request: RealtimeConnectEvent,
    on_realtime_subscribe_request: RealtimeSubscribeEvent,
    on_realtime_message_send: RealtimeMessageEvent,

    // files
    on_file_download_request: FileDownloadEvent,
    on_file_token_request: FileTokenEvent,

    // backups
    on_backup_create: BackupCreateEvent,
    on_backup_restore: BackupRestoreEvent,

    // settings
    on_settings_list_request: SettingsListEvent,
    on_settings_update_request: SettingsUpdateEvent,
    on_settings_reload: SettingsReloadEvent,

    // batch
    on_batch_request: BatchRequestEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestEvent {
        hook_chain: Chain<Self>,
        tags: Vec<String>,
        log: Arc<Mutex<Vec<&'static str>>>,
    }
    crate::impl_event!(TestEvent, tags);

    fn event(log: &Arc<Mutex<Vec<&'static str>>>) -> TestEvent {
        TestEvent {
            log: log.clone(),
            ..Default::default()
        }
    }

    fn push(name: &'static str) -> impl for<'a> Fn(&'a mut TestEvent) -> BoxFuture<'a, HookResult> {
        move |e: &mut TestEvent| {
            Box::pin(async move {
                e.log.lock().unwrap().push(name);
                e.next().await
            })
        }
    }

    #[tokio::test]
    async fn handlers_run_in_priority_order_then_the_finalizer() {
        let hook: Hook<TestEvent> = Hook::new();
        hook.bind(Handler::new(push("late")).with_priority(10));
        hook.bind(Handler::new(push("early")).with_priority(-5));
        hook.bind_func(push("default"));

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        hook.trigger(&mut e, |e| {
            Box::pin(async move {
                e.log.lock().unwrap().push("finalizer");
                Ok(())
            })
        })
        .await
        .unwrap();
        assert_eq!(
            *log.lock().unwrap(),
            ["early", "default", "late", "finalizer"]
        );
    }

    #[tokio::test]
    async fn a_handler_that_skips_next_stops_the_chain_and_the_finalizer() {
        let hook: Hook<TestEvent> = Hook::new();
        hook.bind_func(|e: &mut TestEvent| {
            Box::pin(async move {
                e.log.lock().unwrap().push("stopper");
                Ok(()) // deliberately no `next()`
            })
        });
        hook.bind(Handler::new(push("never")).with_priority(1));

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        hook.trigger(&mut e, |e| {
            Box::pin(async move {
                e.log.lock().unwrap().push("finalizer");
                Ok(())
            })
        })
        .await
        .unwrap();
        assert_eq!(*log.lock().unwrap(), ["stopper"]);
    }

    #[tokio::test]
    async fn an_error_aborts_and_propagates() {
        let hook: Hook<TestEvent> = Hook::new();
        hook.bind_func(|_: &mut TestEvent| Box::pin(async { Err(AppError::forbidden("nope")) }));
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        let err = hook
            .trigger(&mut e, |e| {
                Box::pin(async move {
                    e.log.lock().unwrap().push("finalizer");
                    Ok(())
                })
            })
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403);
        assert!(log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unbind_removes_by_id_and_ids_replace() {
        let hook: Hook<TestEvent> = Hook::new();
        hook.bind(Handler::new(push("a")).with_id("x"));
        hook.bind(Handler::new(push("b")).with_id("x"));
        assert_eq!(hook.len(), 1, "same id replaces");

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        hook.trigger_bare(&mut e).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["b"]);

        assert_eq!(hook.unbind("x"), 1);
        assert_eq!(hook.unbind("x"), 0);
        assert!(hook.is_empty());
    }

    #[tokio::test]
    async fn tagged_handlers_only_fire_for_matching_events() {
        let hook: Hook<TestEvent> = Hook::new();
        hook.bind(Handler::new(push("posts-only")).with_tags(["posts"]));
        hook.bind_func(push("all"));

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        e.tags = vec!["users".into()];
        hook.trigger_bare(&mut e).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["all"]);

        log.lock().unwrap().clear();
        let mut e = event(&log);
        e.tags = vec!["posts".into(), "pbc_1".into()];
        hook.trigger_bare(&mut e).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["posts-only", "all"]);
    }

    #[tokio::test]
    async fn registering_during_a_trigger_takes_effect_next_time() {
        let hook: Arc<Hook<TestEvent>> = Arc::new(Hook::new());
        let inner = hook.clone();
        hook.bind_func(move |e: &mut TestEvent| {
            let inner = inner.clone();
            Box::pin(async move {
                inner.bind_func(push("added"));
                e.log.lock().unwrap().push("first");
                e.next().await
            })
        });

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut e = event(&log);
        hook.trigger_bare(&mut e).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["first"]);

        log.lock().unwrap().clear();
        let mut e = event(&log);
        hook.trigger_bare(&mut e).await.unwrap();
        assert_eq!(*log.lock().unwrap(), ["first", "added"]);
    }
}
