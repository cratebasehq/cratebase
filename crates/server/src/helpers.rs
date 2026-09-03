use cratebase_core::{AppError, Collection};
use cratebase_db::collections;

use crate::http_error::ApiError;
use crate::state::AppState;

/// Collections are addressed by id or by name in every route (`:collection`
/// path param), matching PocketBase's API ergonomics.
pub async fn load_collection(app: &AppState, id_or_name: &str) -> Result<Collection, ApiError> {
    let by_name = collections::get_collection_by_name(&app.db, id_or_name).await;
    let collection = match by_name {
        Ok(c) => c,
        Err(_) => collections::get_collection_by_id(&app.db, id_or_name)
            .await
            .map_err(|_| ApiError(AppError::NotFound(format!("collection '{id_or_name}' not found"))))?,
    };
    Ok(collection)
}

/// Storage key for a record's uploaded file. Namespacing by collection id
/// and record id keeps files from colliding across collections/records and
/// means deleting a record's directory (best-effort on record delete) is a
/// single prefix.
pub fn file_key(collection_id: &str, record_id: &str, filename: &str) -> String {
    format!("{collection_id}/{record_id}/{filename}")
}

/// Turn a user-supplied filename into a safe, collision-resistant stored
/// name: strip any path components (defense against `../` traversal) and
/// prefix a short random token so re-uploading a same-named file never
/// overwrites the previous one.
pub fn unique_filename(original: &str) -> String {
    let base = original.rsplit(['/', '\\']).next().unwrap_or(original);
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (base, None),
    };
    let token = cratebase_core::new_id();
    let token = &token[..10];
    match ext {
        Some(ext) => format!("{stem}_{token}.{ext}"),
        None => format!("{stem}_{token}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_filename_strips_path_and_keeps_extension() {
        let name = unique_filename("../../etc/passwd.png");
        assert!(name.starts_with("passwd_"));
        assert!(name.ends_with(".png"));
        assert!(!name.contains('/'));
    }

    #[test]
    fn unique_filename_handles_no_extension() {
        let name = unique_filename("README");
        assert!(name.starts_with("README_"));
        assert!(!name.contains('.'));
    }
}
