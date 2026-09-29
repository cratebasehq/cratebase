import { useState, type FormEvent } from "react";
import { useNavigate, Link } from "react-router-dom";
import { CratebaseError } from "@cratebase/client";
import { cb } from "@/lib/cratebase";
import { OAuthButtons } from "@/components/OAuthButtons";
import { TotpChallenge } from "@/components/TotpChallenge";

type Tab = "password" | "otp" | "magic-link";

export function SignIn() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>("password");
  const [mfaId, setMfaId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  // Password tab state
  const [identity, setIdentity] = useState("demo@example.com");
  const [password, setPassword] = useState("password123");

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
      <div className="card mx-auto mt-24 w-full max-w-sm">
        <h1 className="mb-4 text-xl font-semibold">Two-factor verification</h1>
        <TotpChallenge mfaId={mfaId} onSuccess={() => navigate("/dashboard")} />
      </div>
    );
  }

  return (
    <div className="card mx-auto mt-24 w-full max-w-sm">
      <h1 className="mb-1 text-xl font-semibold">Sign in</h1>
      <p className="mb-4 text-sm text-black/60 dark:text-white/60">
        No account? <Link to="/sign-up" className="underline">Create one</Link>.
      </p>

      <div className="mb-4 flex gap-1 rounded-lg bg-black/5 p-1 text-sm dark:bg-white/10">
        {(["password", "otp", "magic-link"] as const).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => {
              setTab(t);
              setError(null);
            }}
            className={`flex-1 rounded-md px-2 py-1.5 transition-colors ${
              tab === t ? "bg-white shadow-sm dark:bg-black/40" : "text-black/50 dark:text-white/50"
            }`}
          >
            {t === "password" ? "Password" : t === "otp" ? "Email code" : "Magic link"}
          </button>
        ))}
      </div>

      {tab === "password" && (
        <form onSubmit={handlePasswordSubmit} className="flex flex-col gap-3">
          <input
            className="input"
            type="email"
            placeholder="you@example.com"
            value={identity}
            onChange={(e) => setIdentity(e.target.value)}
            required
          />
          <input
            className="input"
            type="password"
            placeholder="Password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
          <button type="submit" className="btn-primary" disabled={pending}>
            {pending ? "Signing in…" : "Sign in"}
          </button>
        </form>
      )}

      {tab === "otp" &&
        (otpId === null ? (
          <form onSubmit={handleOtpRequest} className="flex flex-col gap-3">
            <input
              className="input"
              type="email"
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
          <form onSubmit={handleOtpConfirm} className="flex flex-col gap-3">
            <p className="text-sm text-black/60 dark:text-white/60">
              We sent a code to {otpEmail}. In dev, check the mail inbox printed by <code>cratebase dev</code>.
            </p>
            <input
              className="input"
              inputMode="numeric"
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
          <p className="text-sm text-black/60 dark:text-white/60">
            Check your inbox for a sign-in link. In dev, check the mail inbox printed by <code>cratebase dev</code>.
          </p>
        ) : (
          <form onSubmit={handleMagicLinkRequest} className="flex flex-col gap-3">
            <input
              className="input"
              type="email"
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

      {error && <p className="mt-3 text-sm text-red-500">{error}</p>}

      <div className="mt-5">
        <OAuthButtons />
      </div>
    </div>
  );
}
