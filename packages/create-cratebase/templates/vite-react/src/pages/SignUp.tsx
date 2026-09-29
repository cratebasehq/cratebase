import { useState, type FormEvent } from "react";
import { useNavigate, Link } from "react-router-dom";
import { cb } from "@/lib/cratebase";
import { OAuthButtons } from "@/components/OAuthButtons";

export function SignUp() {
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await cb.auth.signUp({ email, password, passwordConfirm: password, name });
      navigate("/dashboard");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-up failed.");
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="card mx-auto mt-24 w-full max-w-sm">
      <h1 className="mb-1 text-xl font-semibold">Create an account</h1>
      <p className="mb-4 text-sm text-black/60 dark:text-white/60">
        Already have one? <Link to="/sign-in" className="underline">Sign in</Link>.
      </p>
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <input className="input" placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} />
        <input
          className="input"
          type="email"
          placeholder="you@example.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
        />
        <input
          className="input"
          type="password"
          placeholder="Password (min 8 characters)"
          minLength={8}
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
        />
        {error && <p className="text-sm text-red-500">{error}</p>}
        <button type="submit" className="btn-primary" disabled={pending}>
          {pending ? "Creating account…" : "Create account"}
        </button>
      </form>
      <div className="mt-5">
        <OAuthButtons />
      </div>
    </div>
  );
}
