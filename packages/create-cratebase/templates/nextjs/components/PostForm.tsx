"use client";

import { useRef, useState, type FormEvent } from "react";
import { useMutation, useUpload, useAuth } from "@/lib/cratebase";

export function PostForm() {
  const { user } = useAuth();
  const { create, pending, error } = useMutation("posts");
  const { upload, pending: uploading, progress } = useUpload();
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!user) return;

    const file = fileRef.current?.files?.[0];
    if (file) {
      // A file upload for a *new* record: the presigned token is reserved
      // against `result.recordId`, so the create() that follows must reuse
      // it as the record's own `id` — see the useUpload doc comment.
      const result = await upload(file, { collection: "posts", field: "cover" });
      await create({ id: result.recordId, title, content, owner: user.id, cover: result.token });
    } else {
      await create({ title, content, owner: user.id });
    }

    setTitle("");
    setContent("");
    if (fileRef.current) fileRef.current.value = "";
  }

  return (
    <form onSubmit={handleSubmit} className="card flex flex-col gap-3">
      <h2 className="text-sm font-semibold uppercase tracking-wide text-black/50 dark:text-white/50">
        New note
      </h2>
      <input className="input" placeholder="Title" value={title} onChange={(e) => setTitle(e.target.value)} required />
      <textarea
        className="input min-h-24 resize-y"
        placeholder="Write something…"
        value={content}
        onChange={(e) => setContent(e.target.value)}
      />
      <input ref={fileRef} type="file" accept="image/*" className="text-sm" />
      {uploading && <p className="text-xs text-black/50">Uploading cover… {Math.round(progress * 100)}%</p>}
      {error !== null && <p className="text-sm text-red-500">{String(error)}</p>}
      <button type="submit" className="btn-primary self-start" disabled={pending || uploading}>
        {pending || uploading ? "Saving…" : "Add note"}
      </button>
    </form>
  );
}
