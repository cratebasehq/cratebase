import { cb } from "@/lib/api";

/**
 * Uploads `file` to the `_emailAssets` system collection (a single
 * `file` field, image mime types only, 8MB cap — see
 * `cratebase_core::collection::Collection::default_system_collections`'s
 * doc comment on it) and returns a URL a sent email's `<img src>` can
 * actually use.
 *
 * Deliberately **not** `cb.files.url(...)` — that builds a path
 * relative to the dashboard client's own `baseUrl` (`"/"` in
 * production, see `@/lib/api`'s module doc), which is meaningless once
 * the HTML leaves this browser. `appUrl` (`settings.meta.appURL`) is
 * the actual public address of this Cratebase instance, so the URL is
 * built from that instead, manually, in the same shape
 * `crates/server/src/routes/files.rs` serves: `/api/files/{collectionId}/
 * {recordId}/{filename}`. The `file` field is deliberately not
 * `protected`, so that endpoint serves it to anyone with the link — no
 * session, which is exactly what an emailed image needs.
 */
export async function uploadEmailImage(file: File, appUrl: string): Promise<{ url: string }> {
  const formData = new FormData();
  formData.append("file", file);
  const record = await cb.collection("_emailAssets").create(formData);
  const filename = record["file"];
  if (typeof filename !== "string" || !filename) {
    throw new Error("Upload succeeded but the server returned no filename.");
  }
  const base = appUrl.trim().replace(/\/+$/, "");
  const path = `/api/files/${encodeURIComponent(record.collectionId)}/${encodeURIComponent(record.id)}/${encodeURIComponent(filename)}`;
  return { url: base ? `${base}${path}` : path };
}
