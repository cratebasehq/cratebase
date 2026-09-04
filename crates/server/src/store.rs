//! `App::store()`: a process-wide typed side-table, PocketBase's
//! `app.Store()`.
//!
//! Plugins need somewhere to keep state that outlives a request but does
//! not belong in the database (a connection pool, a compiled template
//! set, a queue handle). Keying by [`TypeId`] rather than by string means
//! two plugins cannot collide unless they genuinely share a type, and the
//! value comes back already typed.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

type AnyValue = Arc<dyn Any + Send + Sync>;

#[derive(Default, Clone)]
pub struct Store {
    inner: Arc<RwLock<HashMap<TypeId, AnyValue>>>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) the value stored for `T`, returning the
    /// previous one.
    pub fn set<T: Any + Send + Sync>(&self, value: T) -> Option<Arc<T>> {
        let previous = self
            .inner
            .write()
            .expect("store poisoned")
            .insert(TypeId::of::<T>(), Arc::new(value));
        previous.and_then(downcast::<T>)
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner
            .read()
            .expect("store poisoned")
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(downcast::<T>)
    }

    /// The stored value for `T`, inserting `f()`'s result if absent.
    pub fn get_or_insert_with<T: Any + Send + Sync>(&self, f: impl FnOnce() -> T) -> Arc<T> {
        if let Some(existing) = self.get::<T>() {
            return existing;
        }
        let mut guard = self.inner.write().expect("store poisoned");
        // Re-check: another thread may have raced us to the write lock.
        if let Some(existing) = guard
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(downcast::<T>)
        {
            return existing;
        }
        let value = Arc::new(f());
        guard.insert(TypeId::of::<T>(), value.clone());
        value
    }

    pub fn remove<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner
            .write()
            .expect("store poisoned")
            .remove(&TypeId::of::<T>())
            .and_then(downcast::<T>)
    }

    pub fn has<T: Any + Send + Sync>(&self) -> bool {
        self.inner
            .read()
            .expect("store poisoned")
            .contains_key(&TypeId::of::<T>())
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("store poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn downcast<T: Any + Send + Sync>(value: AnyValue) -> Option<Arc<T>> {
    value.downcast::<T>().ok()
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("entries", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Counter(u32);
    #[derive(Debug, PartialEq)]
    struct Label(String);

    #[test]
    fn values_are_keyed_by_type() {
        let store = Store::new();
        assert!(store.get::<Counter>().is_none());
        store.set(Counter(1));
        store.set(Label("x".into()));
        assert_eq!(*store.get::<Counter>().unwrap(), Counter(1));
        assert_eq!(*store.get::<Label>().unwrap(), Label("x".into()));
        assert_eq!(store.len(), 2);

        assert_eq!(*store.set(Counter(2)).unwrap(), Counter(1));
        assert_eq!(*store.get::<Counter>().unwrap(), Counter(2));

        assert_eq!(*store.remove::<Counter>().unwrap(), Counter(2));
        assert!(!store.has::<Counter>());
        assert!(store.has::<Label>());
    }

    #[test]
    fn get_or_insert_with_runs_once() {
        let store = Store::new();
        let a = store.get_or_insert_with(|| Counter(7));
        let b = store.get_or_insert_with(|| Counter(9));
        assert_eq!(*a, Counter(7));
        assert_eq!(*b, Counter(7));
    }
}
