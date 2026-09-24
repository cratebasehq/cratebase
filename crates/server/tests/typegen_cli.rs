//! `cratebase typegen` — writes the local schema's TypeScript types to a
//! file (default) or stdout (`-o -`). Runs the real binary against a
//! fresh temp data directory; `App::bootstrap` alone (no `serve`) is
//! enough to seed the built-in `users` collection, so there's no need to
//! `schema push` anything first.

use std::process::Command;

fn cratebase_bin() -> &'static str {
    env!("CARGO_BIN_EXE_cratebase")
}

#[test]
fn writes_to_the_default_file_when_no_out_flag_is_given() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = Command::new(cratebase_bin())
        .args(["typegen", "--dir"])
        .arg(dir.path())
        .current_dir(dir.path())
        .output()
        .expect("run cratebase typegen");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let generated_path = dir.path().join("cratebase-types.d.ts");
    let generated = std::fs::read_to_string(&generated_path).expect("read generated types");
    assert!(generated.contains("export type UsersRecord = {"));
    assert!(generated.contains("export type Schema = {"));
    assert!(generated.contains("export type SchemaCreate = {"));
}

#[test]
fn dash_out_writes_to_stdout_instead_of_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = Command::new(cratebase_bin())
        .args(["typegen", "--dir"])
        .arg(dir.path())
        .args(["-o", "-"])
        .output()
        .expect("run cratebase typegen -o -");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("export type UsersRecord = {"));
    assert!(!dir.path().join("cratebase-types.d.ts").exists());
}

#[test]
fn dash_o_writes_to_a_named_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out_path = dir.path().join("custom-types.d.ts");
    let output = Command::new(cratebase_bin())
        .args(["typegen", "--dir"])
        .arg(dir.path())
        .arg("-o")
        .arg(&out_path)
        .output()
        .expect("run cratebase typegen -o <path>");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = std::fs::read_to_string(&out_path).expect("read generated types");
    assert!(generated.contains("export type UsersRecord = {"));
}
