//! Process-wide bounded cache of parsed expressions. API rules are parsed
//! on every request otherwise; the AST for a given source string never
//! changes, so a small LRU keyed by the source removes that cost.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::ast::Expr;
use crate::error::FilterError;
use crate::parser::Parser;

/// Maximum number of distinct filter strings kept in memory.
pub const CACHE_CAPACITY: usize = 1024;

/// A minimal LRU: every access stamps the entry with a monotonically
/// increasing tick, and inserting into a full cache evicts the entry with
/// the smallest tick. Eviction is O(n) but only happens once the cache is
/// full and misses, which is rare in steady state (rules are a fixed set).
struct Lru {
    map: HashMap<String, (Arc<Expr>, u64)>,
    tick: u64,
    capacity: usize,
}

impl Lru {
    fn new(capacity: usize) -> Self {
        Lru {
            map: HashMap::with_capacity(capacity.min(64)),
            tick: 0,
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<Expr>> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.map.get_mut(key)?;
        entry.1 = tick;
        Some(Arc::clone(&entry.0))
    }

    fn insert(&mut self, key: String, value: Arc<Expr>) {
        if self.map.len() >= self.capacity && !self.map.contains_key(&key) {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, tick))| *tick)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        self.tick += 1;
        self.map.insert(key, (value, self.tick));
    }
}

fn cache() -> &'static Mutex<Lru> {
    static CACHE: OnceLock<Mutex<Lru>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Lru::new(CACHE_CAPACITY)))
}

/// Parse `src`, returning a shared AST from the global cache when the same
/// source was parsed before. Parse failures are not cached.
pub fn parse_cached(src: &str) -> Result<Arc<Expr>, FilterError> {
    {
        let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = guard.get(src) {
            return Ok(hit);
        }
    }
    let expr = Arc::new(Parser::parse(src)?);
    let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
    // Another thread may have raced us; prefer the stored instance so
    // repeated calls hand out the same Arc.
    if let Some(hit) = guard.get(src) {
        return Ok(hit);
    }
    guard.insert(src.to_string(), Arc::clone(&expr));
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut lru = Lru::new(2);
        let e = Arc::new(Parser::parse("a = 1").unwrap());
        lru.insert("a".into(), e.clone());
        lru.insert("b".into(), e.clone());
        assert!(lru.get("a").is_some()); // touch a → b is now the oldest
        lru.insert("c".into(), e);
        assert!(lru.get("b").is_none());
        assert!(lru.get("a").is_some());
        assert!(lru.get("c").is_some());
    }
}
