import { useEffect, useState } from "react";
import { cb, describeError, useAuth, useRecord, useRecords } from "../cratebase.js";
import { useTeamMembers } from "../hooks/useTeamMembers.js";
import { Avatar } from "./Avatar.js";

const ALL_LABELS = ["bug", "feature", "chore", "urgent", "design"] as const;
const IMAGE_EXT = /\.(png|jpe?g|gif|webp|svg)$/i;

export function CardDetail({ cardId, onClose }: { cardId: string; onClose: () => void }) {
  const { user } = useAuth();
  const { record: card, deleted } = useRecord("cards", cardId, { expand: "assigneeRef" });
  const members = useTeamMembers(card?.teamRef ?? null);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");

  useEffect(() => {
    if (card) {
      setTitle(card.title);
      setDescription(card.description ?? "");
    }
  }, [card?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const { records: comments } = useRecords("comments", {
    filter: `cardRef = "${cardId}"`,
    sort: "created",
    expand: "authorRef",
    enabled: !!card,
  });
  const { records: attachments } = useRecords("attachments", {
    filter: `cardRef = "${cardId}"`,
    expand: "uploaderRef",
    enabled: !!card,
  });

  const [commentBody, setCommentBody] = useState("");
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);

  async function save(patch: Record<string, unknown>) {
    try {
      await cb.collection("cards").update(cardId, patch);
      setError("");
    } catch (err) {
      setError(describeError(err));
    }
  }

  async function postComment(e: React.FormEvent) {
    e.preventDefault();
    if (!commentBody.trim() || !card || !user) return;
    const body = commentBody.trim();
    setCommentBody("");
    try {
      await cb.collection("comments").create({ cardRef: cardId, teamRef: card.teamRef, authorRef: user.id, body });
    } catch (err) {
      setError(describeError(err));
    }
  }

  async function uploadFile(file: File) {
    if (!card || !user) return;
    setUploading(true);
    const form = new FormData();
    form.append("cardRef", cardId);
    form.append("teamRef", card.teamRef);
    form.append("uploaderRef", user.id);
    form.append("file", file);
    try {
      await cb.collection("attachments").create(form);
    } catch (err) {
      setError(describeError(err));
    } finally {
      setUploading(false);
    }
  }

  if (deleted) {
    return (
      <Panel onClose={onClose}>
        <p className="text-sm text-ink/60 dark:text-paper/60">This card was deleted.</p>
      </Panel>
    );
  }
  if (!card) return null;

  return (
    <Panel onClose={onClose}>
      <input
        className="w-full border-none bg-transparent font-display text-xl font-semibold outline-none"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={() => title.trim() && title !== card.title && save({ title: title.trim() })}
      />
      <span className="mt-1 block font-mono text-[11px] text-ink/35 dark:text-paper/35">
        CB-{card.id.slice(0, 4).toUpperCase()}
      </span>

      {error && <p className="mt-3 text-sm text-crate">{error}</p>}

      <div className="mt-5 grid grid-cols-[80px_1fr] items-start gap-y-3 text-sm">
        <span className="pt-1.5 text-ink/50 dark:text-paper/50">Assignee</span>
        <select
          className="input"
          value={card.assigneeRef ?? ""}
          onChange={(e) => save({ assigneeRef: e.target.value || null })}
        >
          <option value="">Unassigned</option>
          {members.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name || m.email}
            </option>
          ))}
        </select>

        <span className="pt-1.5 text-ink/50 dark:text-paper/50">Labels</span>
        <div className="flex flex-wrap gap-1.5">
          {ALL_LABELS.map((label) => {
            const active = (card.labels ?? []).includes(label);
            return (
              <button
                key={label}
                type="button"
                onClick={() =>
                  save({ labels: active ? (card.labels ?? []).filter((l) => l !== label) : [...(card.labels ?? []), label] })
                }
                className={
                  "chip border " + (active ? "border-crate bg-crate-50 text-crate dark:bg-crate-dark/15 dark:text-crate-dark" : "border-slate/50 text-ink/50 dark:border-slate-dark dark:text-paper/50")
                }
              >
                {label}
              </button>
            );
          })}
        </div>

        <span className="pt-1.5 text-ink/50 dark:text-paper/50">Due</span>
        <input
          type="date"
          className="input w-40"
          value={card.dueAt ? card.dueAt.slice(0, 10) : ""}
          onChange={(e) => save({ dueAt: e.target.value ? new Date(e.target.value).toISOString() : null })}
        />
      </div>

      <div className="mt-6">
        <h3 className="text-xs font-medium uppercase tracking-wide text-ink/40 dark:text-paper/40">Description</h3>
        <textarea
          className="input mt-2 w-full"
          rows={4}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onBlur={() => description !== (card.description ?? "") && save({ description })}
          placeholder="Add a description…"
        />
      </div>

      <div className="mt-6">
        <h3 className="text-xs font-medium uppercase tracking-wide text-ink/40 dark:text-paper/40">Attachments</h3>
        <div className="mt-2 flex flex-wrap gap-2">
          {attachments.map((a) => {
            const filename = a.file;
            const url = filename ? cb.files.url(a, filename, { thumb: "100x100" }) : "";
            const isImage = filename && IMAGE_EXT.test(filename);
            return (
              <a key={a.id} href={filename ? cb.files.url(a, filename) : "#"} target="_blank" rel="noreferrer" className="block">
                {isImage ? (
                  <img src={url} alt={filename} className="h-16 w-16 rounded-control border border-slate/50 object-cover dark:border-slate-dark" />
                ) : (
                  <span className="flex h-16 w-16 items-center justify-center rounded-control border border-slate/50 text-[11px] text-ink/50 dark:border-slate-dark dark:text-paper/50">
                    {filename?.split(".").pop()?.toUpperCase() ?? "FILE"}
                  </span>
                )}
              </a>
            );
          })}
          <label className="flex h-16 w-16 cursor-pointer items-center justify-center rounded-control border border-dashed border-slate/60 text-xs text-ink/40 hover:bg-paper-200 dark:border-slate-dark dark:text-paper/40 dark:hover:bg-ink-600">
            {uploading ? "…" : "+ file"}
            <input
              type="file"
              className="hidden"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) void uploadFile(file);
                e.target.value = "";
              }}
            />
          </label>
        </div>
      </div>

      <div className="mt-6">
        <h3 className="text-xs font-medium uppercase tracking-wide text-ink/40 dark:text-paper/40">Comments</h3>
        <ul className="mt-2 flex flex-col gap-3">
          {comments.map((c) => (
            <li key={c.id} className="flex gap-2">
              <Avatar id={c.authorRef} name={c.expand?.authorRef?.name || c.expand?.authorRef?.email || "?"} size={22} />
              <div className="min-w-0">
                <p className="text-xs text-ink/50 dark:text-paper/50">
                  {c.expand?.authorRef?.name || c.expand?.authorRef?.email} · {new Date(c.created).toLocaleString()}
                </p>
                <p className="text-sm">{c.body}</p>
              </div>
            </li>
          ))}
        </ul>
        <form onSubmit={postComment} className="mt-3 flex gap-2">
          <input className="input flex-1" placeholder="Write a comment…" value={commentBody} onChange={(e) => setCommentBody(e.target.value)} />
          <button type="submit" className="btn-primary">
            Send
          </button>
        </form>
      </div>
    </Panel>
  );
}

function Panel({ children, onClose }: { children: React.ReactNode; onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-20 flex justify-end bg-ink/30" onClick={onClose}>
      <div
        className="h-full w-full max-w-[480px] overflow-y-auto bg-paper-100 p-6 shadow-lift dark:bg-ink-700"
        onClick={(e) => e.stopPropagation()}
      >
        <button type="button" onClick={onClose} className="mb-4 text-sm text-ink/50 hover:text-ink dark:text-paper/50 dark:hover:text-paper">
          ← Close
        </button>
        {children}
      </div>
    </div>
  );
}
