//! End-to-end check that `cratebase_server::typegen::generate`'s output
//! is exactly what `sdk/js/client`'s `createClient<Schema>()` (and the
//! optional `SchemaCreate`/`SchemaUpdate` type params) actually consume
//! — not just that the Rust side compiles, but that real `tsc` accepts
//! the generated `.d.ts` against the real SDK source.
//!
//! Skips gracefully (prints a note, does not fail) when `bun` isn't on
//! `PATH`, or when `bun install`ing the SDK's `typescript` devDependency
//! fails (e.g. no network) — this is the one test in the suite that
//! reaches outside `cargo`, so it must not break `cargo test` on a
//! machine without a JS toolchain.

use std::path::Path;
use std::process::Command;

fn sdk_client_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/js/client")
}

fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensures `sdk/js/client/node_modules/.bin/tsc` exists, installing the
/// package's own `typescript` devDependency if needed. `None` when that
/// fails (no network, say) — the caller should skip, not fail.
fn ensure_tsc() -> Option<std::path::PathBuf> {
    let dir = sdk_client_dir();
    let tsc = dir.join("node_modules/.bin/tsc");
    if tsc.exists() {
        return Some(tsc);
    }
    let status = Command::new("bun")
        .arg("install")
        .current_dir(&dir)
        .status()
        .ok()?;
    if !status.success() || !tsc.exists() {
        return None;
    }
    Some(tsc)
}

#[test]
fn generated_types_typecheck_against_the_real_sdk() {
    if !bun_available() {
        eprintln!("skipping generated_types_typecheck_against_the_real_sdk: bun not on PATH");
        return;
    }
    let Some(tsc) = ensure_tsc() else {
        eprintln!(
            "skipping generated_types_typecheck_against_the_real_sdk: could not install sdk/js/client's typescript devDependency"
        );
        return;
    };

    // A schema covering enough surface to exercise Schema/SchemaCreate/
    // SchemaUpdate and a typed `expand`: an auth collection, a base
    // collection with a select and a relation back to it.
    let authors = cratebase_core::Collection::default_users();
    let mut posts = cratebase_core::Collection::new("posts", cratebase_core::CollectionType::Base);
    let mut author = cratebase_core::Field::new(
        "author",
        cratebase_core::FieldKind::Relation {
            collection_id: authors.id.clone(),
            cascade_delete: false,
            min_select: 0,
            max_select: 1,
        },
    );
    author.required = true;
    let status = cratebase_core::Field::new(
        "status",
        cratebase_core::FieldKind::Select {
            values: vec!["draft".into(), "published".into()],
            max_select: 1,
        },
    );
    let pos = posts.fields.len() - 2;
    posts.fields.splice(pos..pos, [author, status]);

    let generated = cratebase_server::typegen::generate([&authors, &posts]);

    let dir = tempfile::tempdir().expect("temp dir");
    let types_path = dir.path().join("cratebase-types.d.ts");
    std::fs::write(&types_path, &generated).expect("write generated types");

    // `moduleResolution: bundler` resolves a `.js`-suffixed specifier
    // back to the sibling `.ts` source — the same convention this SDK's
    // own modules use to import each other (e.g. `from "./transport.js"`
    // in `records.ts`).
    let sdk_index = sdk_client_dir().join("src/index.js");
    let consumer_path = dir.path().join("consumer.ts");
    std::fs::write(
        &consumer_path,
        format!(
            r#"import {{ createClient }} from "{}";
import type {{ Schema, SchemaCreate, SchemaUpdate }} from "./cratebase-types.js";

const client = createClient<Schema>("http://localhost:8090");
client.collection("posts").create({{ status: "draft" }});

const typedClient = createClient<Schema, SchemaCreate, SchemaUpdate>("http://localhost:8090");
typedClient.collection("posts").create({{ author: "abc123", status: "published" }});

async function read() {{
  const post = await client.collection("posts").one("abc123", {{ expand: "author" }});
  const author = post.expand?.author;
  if (author) {{
    const email: string = author.email;
    console.log(email);
  }}
}}
void read();
"#,
            sdk_index.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("write consumer.ts");

    let output = Command::new(&tsc)
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "es2022",
            "--module",
            "esnext",
            "--moduleResolution",
            "bundler",
        ])
        .arg(&consumer_path)
        .output()
        .expect("run tsc");

    assert!(
        output.status.success(),
        "generated types failed to typecheck against sdk/js/client:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
