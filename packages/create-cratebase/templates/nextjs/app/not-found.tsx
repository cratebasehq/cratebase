import Link from "next/link";

export default function NotFound() {
  return (
    <main className="mx-auto flex min-h-screen max-w-sm flex-col items-center justify-center gap-4 px-6 text-center">
      <p className="text-5xl font-bold text-[var(--color-accent)]">404</p>
      <h1 className="text-xl font-semibold">Page not found</h1>
      <Link href="/" className="btn-secondary">
        Back home
      </Link>
    </main>
  );
}
