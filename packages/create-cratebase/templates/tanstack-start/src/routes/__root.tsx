import type { ReactNode } from "react";
import { HeadContent, Link, Outlet, Scripts, createRootRoute, useRouter } from "@tanstack/react-router";
import { getUserFn, signOutFn, type SessionUser } from "~/server/auth.functions";
import "~/styles.css";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: "{{PROJECT_NAME}}" },
      { name: "description", content: "A TanStack Start app on Cratebase." },
      { name: "theme-color", content: "#f6f4ef", media: "(prefers-color-scheme: light)" },
      { name: "theme-color", content: "#16181d", media: "(prefers-color-scheme: dark)" },
    ],
    links: [{ rel: "icon", href: "/favicon.svg", type: "image/svg+xml" }],
  }),
  // Resolved on the server for every navigation; the session cookie is HttpOnly,
  // so this server function is how the UI learns who is signed in.
  beforeLoad: async () => ({ user: await getUserFn().catch((): SessionUser | null => null) }),
  component: RootComponent,
  notFoundComponent: NotFound,
});

function RootComponent() {
  return (
    <RootDocument>
      <Header />
      <Outlet />
    </RootDocument>
  );
}

function RootDocument({ children }: Readonly<{ children: ReactNode }>) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}

function Header() {
  const { user } = Route.useRouteContext();
  const router = useRouter();

  async function signOut() {
    await signOutFn();
    await router.invalidate();
    await router.navigate({ to: "/" });
  }

  return (
    <header className="mx-auto flex max-w-2xl items-center justify-between gap-4 px-6 py-5">
      <Link to="/" className="font-semibold tracking-tight">
        {"{{PROJECT_NAME}}"}
      </Link>
      <nav aria-label="Main" className="flex items-center gap-2 text-sm">
        {user ? (
          <>
            <Link to="/dashboard" className="btn-secondary">
              Dashboard
            </Link>
            <button type="button" className="btn-secondary" onClick={signOut}>
              Sign out
            </button>
          </>
        ) : (
          <>
            <Link to="/sign-in" className="btn-secondary">
              Sign in
            </Link>
            <Link to="/sign-up" className="btn-primary">
              Create account
            </Link>
          </>
        )}
      </nav>
    </header>
  );
}

function NotFound() {
  return (
    <main className="mx-auto flex max-w-sm flex-col items-center gap-4 px-6 py-24 text-center">
      <p className="text-5xl font-bold text-[var(--color-accent)]">404</p>
      <h1 className="text-xl font-semibold">Page not found</h1>
      <Link to="/" className="btn-secondary">
        Back home
      </Link>
    </main>
  );
}
