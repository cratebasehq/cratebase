"use client";

import { useState } from "react";
import type { RecordModel } from "@cratebase/client";
import { cb, useMutation } from "@/lib/cratebase";
import type { PostsRecord } from "../cratebase-types";

type Post = PostsRecord & RecordModel;

export function PostList({ posts, loading }: { posts: Post[]; loading: boolean }) {
  const { update, remove } = useMutation("posts");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState({ title: "", content: "" });

  function startEdit(post: Post) {
    setEditingId(post.id);
    setDraft({ title: post.title, content: post.content ?? "" });
  }

  async function saveEdit(id: string) {
    await update(id, { title: draft.title, content: draft.content });
    setEditingId(null);
  }

  if (loading) return <p className="text-sm text-black/50">Loading…</p>;
  if (posts.length === 0) return <p className="text-sm text-black/50">No notes yet — add your first one above.</p>;

  return (
    <ul className="flex flex-col gap-3">
      {posts.map((post) => (
        <li key={post.id} className="card">
          {editingId === post.id ? (
            <div className="flex flex-col gap-2">
              <input
                className="input"
                value={draft.title}
                onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
              />
              <textarea
                className="input min-h-20"
                value={draft.content}
                onChange={(e) => setDraft((d) => ({ ...d, content: e.target.value }))}
              />
              <div className="flex gap-2">
                <button type="button" className="btn-primary" onClick={() => saveEdit(post.id)}>
                  Save
                </button>
                <button type="button" className="btn-secondary" onClick={() => setEditingId(null)}>
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <>
              {post.cover && (
                // eslint-disable-next-line @next/next/no-img-element -- a signed/local file URL, not an optimizable static asset
                <img
                  src={cb.files.url(post, post.cover, { thumb: "400x200" })}
                  alt=""
                  className="mb-3 h-40 w-full rounded-lg object-cover"
                />
              )}
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h3 className="font-medium">{post.title}</h3>
                  {post.content && <p className="mt-1 whitespace-pre-wrap text-sm text-black/70 dark:text-white/70">{post.content}</p>}
                </div>
                <div className="flex shrink-0 gap-2">
                  <button type="button" className="btn-secondary !px-2 !py-1 text-xs" onClick={() => startEdit(post)}>
                    Edit
                  </button>
                  <button
                    type="button"
                    className="btn-secondary !px-2 !py-1 text-xs text-red-500"
                    onClick={() => remove(post.id)}
                  >
                    Delete
                  </button>
                </div>
              </div>
            </>
          )}
        </li>
      ))}
    </ul>
  );
}
