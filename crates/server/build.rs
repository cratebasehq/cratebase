//! Guards the one thing that makes `cargo build` enough: the dashboard
//! bundle has to actually be in the tree, because it is compiled into the
//! binary.
//!
//! `rust-embed` fails loudly when the folder is missing, but not when the
//! folder exists and is empty — that produces a binary that starts, serves
//! the API, and renders a blank page at `/_/`. Checking for `index.html`
//! turns that into a build error with something to act on.

use std::path::{Path, PathBuf};

fn main() {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("web")
        .join("admin")
        .join("dist");

    // Rebuild the binary when the bundle changes, or a `bun run
    // admin:build` would leave the embedded copy stale.
    println!("cargo:rerun-if-changed={}", dist.display());
    if let Ok(entries) = std::fs::read_dir(dist.join("assets")) {
        for entry in entries.flatten() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }

    if !dist.join("index.html").exists() {
        panic!(
            "the admin dashboard bundle is missing from {}.\n\
             \n\
             It is committed to the repository, so a normal clone has it and \
             `cargo build` needs nothing else. If you deleted it or are \
             building from a source archive that dropped it, rebuild with:\n\
             \n    bun install && bun run admin:build\n",
            display(&dist)
        );
    }
}

/// Absolute paths in a panic message are noise; show it from the repo root.
fn display(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
