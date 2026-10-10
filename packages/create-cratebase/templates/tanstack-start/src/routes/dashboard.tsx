import { useState, type FormEvent } from "react";
import { createFileRoute, redirect, useRouter } from "@tanstack/react-router";
import { Field } from "~/components/Field";
import { createNoteFn, deleteNoteFn, listMyNotesFn } from "~/server/posts.functions";

export const Route = createFileRoute("/dashboard")({
  // Redirect early for a smooth UX; every server function below is also
  // guarded by `cb.authMiddleware`, which is the real security boundary.
  beforeLoad: ({ context }) => {
    if (!context.user) throw redirect({ to: "/sign-in" });
  },
  loader: () => listMyNotesFn(),
  component: Dashboard,
});

function Dashboard() {
  const { user } = Route.useRouteContext();
  const notes = Route.useLoaderData();
  const router = useRouter();
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function addNote(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await createNoteFn({ data: { title, content } });
      setTitle("");
      setContent("");
      await router.invalidate();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save the note.");
    } finally {
      setPending(false);
    }
  }

  async function removeNote(id: string, noteTitle: string) {
    if (!window.confirm(`Delete “${noteTitle}”?`)) return;
    setError(null);
    try {
      await deleteNoteFn({ data: { id } });
      await router.invalidate();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't delete that note.");
    }
  }

  return (
    <main className="mx-auto flex max-w-2xl flex-col gap-6 px-6 pb-16">
      <div>
        <h1 className="text-lg font-semibold">Your notes</h1>
        <p className="muted text-sm">Signed in as {user?.email}</p>
      </div>

      <form onSubmit={addNote} className="card flex flex-col gap-3" aria-label="New note">
        <Field label="Title" placeholder="What's on your mind?" value={title} onChange={(e) => setTitle(e.target.value)} required />
        <div className="flex flex-col gap-1.5">
          <label htmlFor="note-content" className="text-sm font-medium">
            Content
          </label>
          <textarea
            id="note-content"
            className="input min-h-24 resize-y"
            value={content}
            onChange={(e) => setContent(e.target.value)}
          />
        </div>
        {error && (
          <p role="alert" className="error-text">
            {error}
          </p>
        )}
        <button type="submit" className="btn-primary self-start" disabled={pending}>
          {pending ? "Saving…" : "Add note"}
        </button>
      </form>

      {notes.length === 0 ? (
        <div className="card text-center">
          <p className="font-medium">No notes yet</p>
          <p className="muted mt-1 text-sm">Add your first one above.</p>
        </div>
      ) : (
        <ul className="flex flex-col gap-3">
          {notes.map((note) => (
            <li key={note.id} className="card flex items-start justify-between gap-3">
              <div className="min-w-0">
                <h2 className="font-medium">{note.title}</h2>
                {note.content && <p className="muted mt-1 whitespace-pre-wrap text-sm">{note.content}</p>}
              </div>
              <button
                type="button"
                className="btn-secondary !px-2.5 !py-1 text-xs text-red-600 dark:text-red-400"
                aria-label={`Delete ${note.title}`}
                onClick={() => removeNote(note.id, note.title)}
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
