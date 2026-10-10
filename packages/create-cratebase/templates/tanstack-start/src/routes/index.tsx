import { createFileRoute } from "@tanstack/react-router";
import { listPublicNotesFn } from "~/server/posts.functions";

export const Route = createFileRoute("/")({
  // Runs on the server during SSR through `cb.client()` — no token ever reaches the browser.
  loader: () => listPublicNotesFn(),
  component: Home,
});

function Home() {
  const notes = Route.useLoaderData();

  return (
    <main className="mx-auto max-w-2xl px-6 pb-16">
      <section className="py-10">
        <p className="text-sm font-medium uppercase tracking-widest text-[var(--color-accent)]">Built on Cratebase</p>
        <h1 className="mt-3 text-4xl font-bold tracking-tight sm:text-5xl">{"{{PROJECT_NAME}}"}</h1>
        <p className="muted mt-3">
          Server-rendered with TanStack Start. Sessions are HttpOnly cookies managed by <code>@cratebase/start</code>.
        </p>
      </section>

      <h2 className="muted mb-3 text-sm font-semibold uppercase tracking-wide">Latest notes</h2>
      {notes.length === 0 ? (
        <div className="card text-center">
          <p className="font-medium">Nothing here yet</p>
          <p className="muted mt-1 text-sm">Sign in and add a note from your dashboard — it shows up here.</p>
        </div>
      ) : (
        <ul className="flex flex-col gap-3">
          {notes.map((note) => (
            <li key={note.id} className="card">
              <h3 className="font-medium">{note.title}</h3>
              {note.content && <p className="muted mt-1 whitespace-pre-wrap text-sm">{note.content}</p>}
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
