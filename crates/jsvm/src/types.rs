//! The `types.d.ts` file hook authors reference with
//! `/// <reference path="../pb_data/types.d.ts" />`.

use std::path::Path;

/// TypeScript declarations for the globals this runtime provides.
pub const TYPES_D_TS: &str = include_str!("types.d.ts");

/// Write [`TYPES_D_TS`] to `path`, creating parent directories. The file
/// is only rewritten when its content changed, so editors do not see
/// spurious updates on every start.
pub fn write_types_file(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::read_to_string(path)
        .map(|c| c == TYPES_D_TS)
        .unwrap_or(false)
    {
        return Ok(());
    }
    std::fs::write(path, TYPES_D_TS)
}
