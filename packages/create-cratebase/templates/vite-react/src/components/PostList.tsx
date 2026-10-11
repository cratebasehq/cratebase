import { useState } from "react";
import type { RecordModel } from "@cratebase/client";
import { cb, useMutation } from "@/lib/cratebase";
import { Field } from "@/components/Field";
import type { PostsRecord } from "../../cratebase-types";

type Post = PostsRecord & RecordModel;

interface PostListProps {
  posts: Post[];
  loading: boolean;
  /** Set when the list query itself failed. */
  error?: unknown;
  /** True while a search term is active, so the empty state can say so. */
  searching?: boolean;
}

function message(err: unknown, fallback: string): string {
  return err instanceof Error ? err.message : fallback;
}

export function PostList({ posts, loading, error, searching = false }: PostListProps) {
  const { update, remove } = useMutation("posts");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState({ title: "", content: "" });
  const [actionError, setActionError] = useState<string | null>(null);

  function startEdit(post: Post) {
    setActionError(null);
    setEditingId(post.id);
    setDraft({ title: post.title, content: post.content ?? "" });
  }

  async function saveEdit(id: string) {
    setActionError(null);
    try {
      await update(id, { title: draft.title, content: draft.content });
      setEditingId(null);
    } catch (err) {
      setActionError(message(err, "Couldn't save your changes."));
    }
  }

  async function removePost(post: Post) {
    if (!window.confirm(`Delete “${post.title}”?`)) return;
    setActionError(null);
    try {
      await remove(post.id);
    } catch (err) {
      setActionError(message(err, "Couldn't delete that note."));
    }
  }

  if (error) {
    return (
      <div role="alert" className="card text-center">
        <p className="font-medium">Couldn't load your notes</p>
        <p className="muted mt-1 text-sm">{message(error, "Is the Cratebase server running?")}</p>
      </div>
    );
  }

  if (loading && posts.length === 0) {
    return (
      <div className="flex flex-col gap-3" aria-busy="true" aria-label="Loading notes">
        <div className="skeleton h-24" />
        <div className="skeleton h-24" />
      </div>
    );
  }

  if (posts.length === 0) {
    return (
      <div className="card text-center">
        <p className="font-medium">{searching ? "No notes match your search" : "No notes yet"}</p>
        <p className="muted mt-1 text-sm">
          {searching ? "Try a different word." : "Add your first one with the form above — it shows up here instantly."}
        </p>
      </div>
    );
  }

  return (
    <>
      {actionError && (
        <p role="alert" className="error-text mb-3">
          {actionError}
        </p>
      )}
      <ul className="flex flex-col gap-3">
        {posts.map((post) => (
          <li key={post.id} className="card">
            {editingId === post.id ? (
              <form
                className="flex flex-col gap-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  void saveEdit(post.id);
                }}
              >
                <Field
                  label="Title"
                  value={draft.title}
                  required
                  onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
                />
                <div className="flex flex-col gap-1.5">
                  <label htmlFor={`content-${post.id}`} className="text-sm font-medium">
                    Content
                  </label>
                  <textarea
                    id={`content-${post.id}`}
                    className="input min-h-20"
                    value={draft.content}
                    onChange={(e) => setDraft((d) => ({ ...d, content: e.target.value }))}
                  />
                </div>
                <div className="flex gap-2">
                  <button type="submit" className="btn-primary">
                    Save
                  </button>
                  <button type="button" className="btn-secondary" onClick={() => setEditingId(null)}>
                    Cancel
                  </button>
                </div>
              </form>
            ) : (
              <>
                {post.cover && (
                  <img
                    src={cb.files.url(post, post.cover, { thumb: "400x200" })}
                    alt=""
                    loading="lazy"
                    className="mb-3 h-40 w-full rounded-lg object-cover"
                  />
                )}
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="font-medium">{post.title}</h3>
                    {post.content && <p className="muted mt-1 whitespace-pre-wrap text-sm">{post.content}</p>}
                  </div>
                  <div className="flex shrink-0 gap-2">
                    <button
                      type="button"
                      className="btn-secondary !px-2.5 !py-1 text-xs"
                      aria-label={`Edit ${post.title}`}
                      onClick={() => startEdit(post)}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className="btn-secondary !px-2.5 !py-1 text-xs text-red-600 dark:text-red-400"
                      aria-label={`Delete ${post.title}`}
                      onClick={() => void removePost(post)}
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
    </>
  );
}
