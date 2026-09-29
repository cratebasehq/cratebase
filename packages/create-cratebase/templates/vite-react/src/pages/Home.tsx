import { Link } from "react-router-dom";

export function Home() {
  return (
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col items-center justify-center gap-8 px-6 text-center">
      <div className="space-y-3">
        <p className="text-sm font-medium uppercase tracking-widest text-[var(--color-accent)]">
          create-cratebase
        </p>
        <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">{"{{PROJECT_NAME}}"}</h1>
        <p className="text-base text-black/60 dark:text-white/60">
          A Vite + React app wired up to Cratebase: auth, a realtime dashboard, file uploads, and search — ready
          to build on.
        </p>
      </div>
      <div className="flex flex-wrap items-center justify-center gap-3">
        <Link to="/sign-up" className="btn-primary">
          Create an account
        </Link>
        <Link to="/sign-in" className="btn-secondary">
          Sign in
        </Link>
      </div>
      <p className="text-xs text-black/40 dark:text-white/40">
        Demo login: <code>demo@example.com</code> / <code>password123</code>
      </p>
    </main>
  );
}
