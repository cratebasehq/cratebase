"use client";

import { useRef, useState, type FormEvent } from "react";
import { useMutation, useUpload, useAuth } from "@/lib/cratebase";
import { Field } from "@/components/Field";

export function PostForm() {
  const { user } = useAuth();
  const { create, pending } = useMutation("posts");
  const { upload, pending: uploading, progress } = useUpload();
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!user) return;
    setError(null);

    try {
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
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save the note.");
      return;
    }

    setTitle("");
    setContent("");
    if (fileRef.current) fileRef.current.value = "";
  }

  const busy = pending || uploading;

  return (
    <form onSubmit={handleSubmit} className="card flex flex-col gap-3" aria-label="New note">
      <h2 className="muted text-sm font-semibold uppercase tracking-wide">New note</h2>
      <Field label="Title" placeholder="What's on your mind?" value={title} onChange={(e) => setTitle(e.target.value)} required />
      <div className="flex flex-col gap-1.5">
        <label htmlFor="note-content" className="text-sm font-medium">
          Content
        </label>
        <textarea
          id="note-content"
          className="input min-h-24 resize-y"
          placeholder="Write something…"
          value={content}
          onChange={(e) => setContent(e.target.value)}
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor="note-cover" className="text-sm font-medium">
          Cover image <span className="muted font-normal">(optional)</span>
        </label>
        <input id="note-cover" ref={fileRef} type="file" accept="image/*" className="text-sm" />
      </div>
      {uploading && (
        <p className="muted text-xs" role="status">
          Uploading cover… {Math.round(progress * 100)}%
        </p>
      )}
      {error && (
        <p role="alert" className="error-text">
          {error}
        </p>
      )}
      <button type="submit" className="btn-primary self-start" disabled={busy}>
        {busy ? "Saving…" : "Add note"}
      </button>
    </form>
  );
}
