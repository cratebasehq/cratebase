//! CommonJS module resolution for `require()`.
//!
//! Resolution happens here (pure file-system logic); evaluation and the
//! per-worker module cache live in `prelude.js`.

use std::path::{Path, PathBuf};

use cratebase_core::AppError;
use serde::Serialize;

/// A resolved module, ready for the JavaScript loader.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedModule {
    /// Canonical absolute path (the cache key).
    pub path: String,
    /// The module's directory, for nested relative requires.
    pub dir: String,
    pub source: String,
    /// `true` for `.json` files (parsed instead of evaluated).
    pub json: bool,
}

/// Resolve `spec` as `require()` would from a module living in `from_dir`:
///
/// - `./x`, `../x` and absolute paths are resolved against `from_dir`;
/// - bare names are looked up under `<hooks_dir>/node_modules/<name>` and
///   then `<hooks_dir>/<name>`;
/// - every candidate is tried as-is, with `.js`, with `.json`, as a
///   directory with a `package.json` `main`, and as `<dir>/index.js`.
pub fn load(hooks_dir: &Path, from_dir: &Path, spec: &str) -> Result<LoadedModule, AppError> {
    let candidates: Vec<PathBuf> = if spec.starts_with("./") || spec.starts_with("../") {
        vec![from_dir.join(spec)]
    } else if Path::new(spec).is_absolute() {
        vec![PathBuf::from(spec)]
    } else {
        vec![
            hooks_dir.join("node_modules").join(spec),
            from_dir.join("node_modules").join(spec),
            hooks_dir.join(spec),
        ]
    };

    for base in candidates {
        if let Some(file) = resolve_file(&base) {
            return read(&file);
        }
    }
    Err(AppError::bad_request(format!(
        "Cannot find module '{spec}' (from {})",
        from_dir.display()
    )))
}

fn resolve_file(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    for ext in ["js", "json", "cjs"] {
        let with_ext = PathBuf::from(format!("{}.{ext}", base.display()));
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if base.is_dir() {
        if let Some(main) = package_main(base) {
            if let Some(f) = resolve_file(&base.join(main)) {
                return Some(f);
            }
        }
        let index = base.join("index.js");
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

fn package_main(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&text).ok()?;
    pkg.get("main")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .map(str::to_string)
}

fn read(file: &Path) -> Result<LoadedModule, AppError> {
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let source = std::fs::read_to_string(&canonical).map_err(|e| {
        AppError::internal(format!("cannot read module {}: {e}", canonical.display()))
    })?;
    let json = canonical
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    Ok(LoadedModule {
        path: canonical.to_string_lossy().into_owned(),
        dir: canonical
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source,
        json,
    })
}
