import { useState, type FormEvent } from "react";
import { Link, createFileRoute, redirect, useRouter } from "@tanstack/react-router";
import { Field } from "~/components/Field";
import { signInFn } from "~/server/auth.functions";

export const Route = createFileRoute("/sign-in")({
  beforeLoad: ({ context }) => {
    if (context.user) throw redirect({ to: "/dashboard" });
  },
  component: SignIn,
});

function SignIn() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await signInFn({ data: { email, password } });
      await router.invalidate();
      await router.navigate({ to: "/dashboard" });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-in failed.");
    } finally {
      setPending(false);
    }
  }

  return (
    <main className="card mx-auto my-10 w-full max-w-sm">
      <h1 className="mb-1 text-xl font-semibold">Sign in</h1>
      <p className="muted mb-5 text-sm">
        No account?{" "}
        <Link to="/sign-up" className="underline">
          Create one
        </Link>
        .
      </p>
      <form onSubmit={handleSubmit} className="flex flex-col gap-4">
        <Field
          label="Email"
          type="email"
          autoComplete="username"
          placeholder="you@example.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
        />
        <Field
          label="Password"
          type="password"
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />
        {error && (
          <p role="alert" className="error-text">
            {error}
          </p>
        )}
        <button type="submit" className="btn-primary" disabled={pending}>
          {pending ? "Signing in…" : "Sign in"}
        </button>
        {import.meta.env.DEV && (
          <button
            type="button"
            className="muted text-xs underline"
            onClick={() => {
              setEmail("demo@example.com");
              setPassword("password123");
            }}
          >
            Fill in the seeded demo account
          </button>
        )}
      </form>
    </main>
  );
}
