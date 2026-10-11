"use client";

import Link from "next/link";
import { useAuth } from "@/lib/cratebase";

const FEATURES = [
  ["Auth", "Password, email code, magic link, OAuth and two-factor."],
  ["Realtime", "Notes update live across every open tab."],
  ["Files & search", "Cover-image uploads and full-text search."],
] as const;

export default function Home() {
  const { isValid } = useAuth();

  return (
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col items-center justify-center gap-10 px-6 py-16 text-center">
      <div className="space-y-4">
        <p className="text-sm font-medium uppercase tracking-widest text-[var(--color-accent)]">Built on Cratebase</p>
        <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">{"{{PROJECT_NAME}}"}</h1>
        <p className="muted text-base">
          A Vite + React starter wired up to Cratebase — ready to build on.
        </p>
      </div>
      <div className="flex flex-wrap items-center justify-center gap-3">
        {isValid ? (
          <Link href="/dashboard" className="btn-primary">
            Open dashboard
          </Link>
        ) : (
          <>
            <Link href="/sign-up" className="btn-primary">
              Create an account
            </Link>
            <Link href="/sign-in" className="btn-secondary">
              Sign in
            </Link>
          </>
        )}
      </div>
      <ul className="grid w-full gap-3 text-left sm:grid-cols-3">
        {FEATURES.map(([title, body]) => (
          <li key={title} className="card !p-4">
            <h2 className="text-sm font-semibold">{title}</h2>
            <p className="muted mt-1 text-sm">{body}</p>
          </li>
        ))}
      </ul>
      <p className="muted text-xs">
        Demo login: <code>demo@example.com</code> / <code>password123</code>
      </p>
    </main>
  );
}
