import { useState, type FormEvent } from "react";
import { useNavigate, Link } from "react-router-dom";
import { CratebaseError } from "@cratebase/client";
import { cb } from "@/lib/cratebase";
import { Field } from "@/components/Field";
import { OAuthButtons } from "@/components/OAuthButtons";
import { TotpChallenge } from "@/components/TotpChallenge";

type Tab = "password" | "otp" | "magic-link";

const TABS: { id: Tab; label: string }[] = [
  { id: "password", label: "Password" },
  { id: "otp", label: "Email code" },
  { id: "magic-link", label: "Magic link" },
];

export function SignIn() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>("password");
  const [mfaId, setMfaId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  // Password tab state
  const [identity, setIdentity] = useState("");
  const [password, setPassword] = useState("");

  // OTP tab state
  const [otpEmail, setOtpEmail] = useState("");
  const [otpId, setOtpId] = useState<string | null>(null);
  const [otpCode, setOtpCode] = useState("");

  // Magic link tab state
  const [magicEmail, setMagicEmail] = useState("");
  const [magicSent, setMagicSent] = useState(false);

  function handleMfa(err: unknown): boolean {
    if (err instanceof CratebaseError && err.mfaId) {
      setMfaId(err.mfaId);
      return true;
    }
    return false;
  }

  async function handlePasswordSubmit(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await cb.auth.signIn.password({ identity, password });
      navigate("/dashboard");
    } catch (err) {
      if (!handleMfa(err)) setError(err instanceof Error ? err.message : "Sign-in failed.");
    } finally {
      setPending(false);
    }
  }

  async function handleOtpRequest(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      const { otpId } = await cb.auth.otp.request({ email: otpEmail });
      setOtpId(otpId);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't send a code.");
    } finally {
      setPending(false);
    }
  }

  async function handleOtpConfirm(e: FormEvent) {
    e.preventDefault();
    if (!otpId) return;
    setPending(true);
    setError(null);
    try {
      await cb.auth.signIn.otp({ otpId, code: otpCode });
      navigate("/dashboard");
    } catch (err) {
      if (!handleMfa(err)) setError(err instanceof Error ? err.message : "That code didn't work.");
    } finally {
      setPending(false);
    }
  }

  async function handleMagicLinkRequest(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await cb.auth.magicLink.request({
        email: magicEmail,
        redirectUrl: `${window.location.origin}/auth/magic-link`,
      });
      setMagicSent(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't send a magic link.");
    } finally {
      setPending(false);
    }
  }

  if (mfaId) {
    return (
      <main className="card mx-auto my-16 w-full max-w-sm">
        <h1 className="mb-4 text-xl font-semibold">Two-factor verification</h1>
        <TotpChallenge mfaId={mfaId} onSuccess={() => navigate("/dashboard")} />
      </main>
    );
  }

  return (
    <main className="card mx-auto my-16 w-full max-w-sm">
      <h1 className="mb-1 text-xl font-semibold">Sign in</h1>
      <p className="muted mb-5 text-sm">
        No account?{" "}
        <Link to="/sign-up" className="underline">
          Create one
        </Link>
        .
      </p>

      <div role="tablist" aria-label="Sign-in method" className="mb-5 flex gap-1 rounded-lg bg-black/5 p-1 text-sm dark:bg-white/10">
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            type="button"
            role="tab"
            id={`tab-${id}`}
            aria-selected={tab === id}
            aria-controls="signin-panel"
            onClick={() => {
              setTab(id);
              setError(null);
            }}
            className={`flex-1 rounded-md px-2 py-1.5 transition-colors ${
              tab === id ? "bg-white font-medium shadow-sm dark:bg-black/40" : "muted hover:text-current"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <div role="tabpanel" id="signin-panel" aria-labelledby={`tab-${tab}`}>
        {tab === "password" && (
          <form onSubmit={handlePasswordSubmit} className="flex flex-col gap-4">
            <Field
              label="Email"
              type="email"
              autoComplete="username"
              placeholder="you@example.com"
              value={identity}
              onChange={(e) => setIdentity(e.target.value)}
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
            <button type="submit" className="btn-primary" disabled={pending}>
              {pending ? "Signing in…" : "Sign in"}
            </button>
            {import.meta.env.DEV && (
              <button
                type="button"
                className="muted text-xs underline"
                onClick={() => {
                  setIdentity("demo@example.com");
                  setPassword("password123");
                }}
              >
                Fill in the seeded demo account
              </button>
            )}
          </form>
        )}

        {tab === "otp" &&
          (otpId === null ? (
            <form onSubmit={handleOtpRequest} className="flex flex-col gap-4">
              <Field
                label="Email"
                type="email"
                autoComplete="email"
                placeholder="you@example.com"
                value={otpEmail}
                onChange={(e) => setOtpEmail(e.target.value)}
                required
              />
              <button type="submit" className="btn-primary" disabled={pending}>
                {pending ? "Sending…" : "Send me a code"}
              </button>
            </form>
          ) : (
            <form onSubmit={handleOtpConfirm} className="flex flex-col gap-4">
              <p className="muted text-sm">
                We sent a code to {otpEmail}. In dev, open the Mail inbox in the Cratebase dashboard (<code>localhost:8090/_/</code>).
              </p>
              <Field
                label="Code"
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="123456"
                value={otpCode}
                onChange={(e) => setOtpCode(e.target.value)}
                required
              />
              <button type="submit" className="btn-primary" disabled={pending}>
                {pending ? "Verifying…" : "Verify code"}
              </button>
            </form>
          ))}

        {tab === "magic-link" &&
          (magicSent ? (
            <p className="muted text-sm" role="status">
              Check your inbox for a sign-in link. In dev, open the Mail inbox in the Cratebase dashboard (<code>localhost:8090/_/</code>).
            </p>
          ) : (
            <form onSubmit={handleMagicLinkRequest} className="flex flex-col gap-4">
              <Field
                label="Email"
                type="email"
                autoComplete="email"
                placeholder="you@example.com"
                value={magicEmail}
                onChange={(e) => setMagicEmail(e.target.value)}
                required
              />
              <button type="submit" className="btn-primary" disabled={pending}>
                {pending ? "Sending…" : "Send magic link"}
              </button>
            </form>
          ))}
      </div>

      {error && (
        <p role="alert" className="error-text mt-3">
          {error}
        </p>
      )}

      <div className="mt-5">
        <OAuthButtons />
      </div>
    </main>
  );
}
