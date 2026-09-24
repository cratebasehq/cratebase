import { useState } from "react";
import { cb, describeError, useAuth } from "../cratebase.js";
import { useAuthMethods } from "../hooks/useAuthMethods.js";

type Tab = "password" | "otp";

export function AuthScreen() {
  const { signIn } = useAuth();
  const methods = useAuthMethods();
  const [tab, setTab] = useState<Tab>("password");
  const [error, setError] = useState("");

  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <BrandPanel />
      <div className="flex flex-1 items-center justify-center px-6 py-16">
        <div className="w-full max-w-[360px]">
          <h1 className="font-display text-2xl font-semibold text-ink dark:text-paper">Sign in to team-board</h1>
          <p className="mt-2 text-sm text-ink/60 dark:text-paper/60">
            A shared board for your team. Use the demo account, or create your own.
          </p>

          <div className="mt-8 flex gap-1 rounded-control bg-paper-200 p-1 dark:bg-ink-600">
            <TabButton active={tab === "password"} onClick={() => setTab("password")}>
              Password
            </TabButton>
            <TabButton active={tab === "otp"} onClick={() => setTab("otp")}>
              Email code
            </TabButton>
          </div>

          {error && (
            <div className="mt-4 rounded-control border border-crate/30 bg-crate-50 px-3 py-2 text-sm text-crate dark:border-crate-dark/40 dark:bg-crate-dark/10 dark:text-crate-dark">
              {error}
            </div>
          )}

          <div className="mt-6">
            {tab === "password" ? <PasswordForm onError={setError} /> : <OtpForm onError={setError} />}
          </div>

          {methods && methods.oauth2.enabled && methods.oauth2.providers.length > 0 && (
            <div className="mt-8">
              <div className="flex items-center gap-3 text-xs text-ink/40 dark:text-paper/40">
                <span className="h-px flex-1 bg-slate/60 dark:bg-slate-dark" />
                or continue with
                <span className="h-px flex-1 bg-slate/60 dark:bg-slate-dark" />
              </div>
              <div className="mt-4 flex flex-col gap-2">
                {methods.oauth2.providers.map((p) => (
                  <button
                    key={p.name}
                    type="button"
                    className="rounded-control border border-slate/60 px-4 py-2 text-sm font-medium hover:bg-paper-200 dark:border-slate-dark dark:hover:bg-ink-600"
                    onClick={() => signIn.social({ provider: p.name, mode: "redirect" })}
                  >
                    Continue with {p.displayName || p.name}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function TabButton({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={
        "flex-1 rounded-[4px] px-3 py-1.5 text-sm font-medium transition-colors " +
        (active ? "bg-paper-100 text-ink shadow-sm dark:bg-ink-700 dark:text-paper" : "text-ink/50 dark:text-paper/50")
      }
    >
      {children}
    </button>
  );
}

function PasswordForm({ onError }: { onError: (msg: string) => void }) {
  const { signIn } = useAuth();
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("alice@example.com");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    onError("");
    setPending(true);
    try {
      if (mode === "signup") {
        await cb.auth.signUp({ email, password, passwordConfirm: password, name });
      } else {
        await signIn.password({ identity: email, password });
      }
    } catch (err) {
      onError(describeError(err));
    } finally {
      setPending(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      {mode === "signup" && (
        <Field label="Name">
          <input value={name} onChange={(e) => setName(e.target.value)} required className="input" placeholder="Ada Lovelace" />
        </Field>
      )}
      <Field label="Email">
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
          className="input"
          placeholder="you@example.com"
        />
      </Field>
      <Field label="Password">
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
          minLength={8}
          className="input"
          placeholder="At least 8 characters"
        />
      </Field>
      <button type="submit" disabled={pending} className="btn-primary mt-1">
        {pending ? "Working…" : mode === "signup" ? "Create account" : "Sign in"}
      </button>
      <button
        type="button"
        onClick={() => setMode(mode === "signup" ? "signin" : "signup")}
        className="text-left text-sm text-manifest hover:underline dark:text-manifest-light"
      >
        {mode === "signup" ? "Already have an account? Sign in" : "New here? Create an account"}
      </button>
    </form>
  );
}

function OtpForm({ onError }: { onError: (msg: string) => void }) {
  const { signIn } = useAuth();
  const [email, setEmail] = useState("alice@example.com");
  const [otpId, setOtpId] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [pending, setPending] = useState(false);

  async function requestCode(e: React.FormEvent) {
    e.preventDefault();
    onError("");
    setPending(true);
    try {
      const { otpId } = await cb.auth.otp.request({ email });
      setOtpId(otpId);
    } catch (err) {
      onError(describeError(err));
    } finally {
      setPending(false);
    }
  }

  async function submitCode(e: React.FormEvent) {
    e.preventDefault();
    onError("");
    if (!otpId) return;
    setPending(true);
    try {
      await signIn.otp({ otpId, code });
    } catch (err) {
      onError(describeError(err));
    } finally {
      setPending(false);
    }
  }

  if (!otpId) {
    return (
      <form onSubmit={requestCode} className="flex flex-col gap-3">
        <Field label="Email">
          <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required className="input" />
        </Field>
        <p className="text-xs text-ink/50 dark:text-paper/50">
          We'll email a one-time code — in dev, open the dashboard's Mail inbox (Settings → Mail inbox) to read it.
        </p>
        <button type="submit" disabled={pending} className="btn-primary mt-1">
          {pending ? "Sending…" : "Send code"}
        </button>
      </form>
    );
  }

  return (
    <form onSubmit={submitCode} className="flex flex-col gap-3">
      <Field label="Code">
        <input
          value={code}
          onChange={(e) => setCode(e.target.value)}
          required
          className="input font-mono tracking-widest"
          placeholder="123456"
          autoFocus
        />
      </Field>
      <button type="submit" disabled={pending} className="btn-primary mt-1">
        {pending ? "Verifying…" : "Verify and sign in"}
      </button>
      <button type="button" onClick={() => setOtpId(null)} className="text-left text-sm text-manifest hover:underline dark:text-manifest-light">
        Use a different email
      </button>
    </form>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="font-medium text-ink/80 dark:text-paper/80">{label}</span>
      {children}
    </label>
  );
}

function BrandPanel() {
  return (
    <div className="flex flex-col justify-between bg-ink px-10 py-12 text-paper md:w-[42%]">
      <div className="flex items-center gap-2 font-display text-lg font-semibold">
        <CrateMark />
        team-board
      </div>
      <div>
        <CrateStack />
        <p className="mt-8 max-w-xs font-display text-2xl leading-snug">Ship work, not status updates.</p>
        <p className="mt-3 max-w-xs text-sm text-paper/60">
          Boards, assignees, comments and semantic search — built on Cratebase.
        </p>
      </div>
      <p className="text-xs text-paper/40">Demo: alice@example.com / password123</p>
    </div>
  );
}

function CrateMark() {
  return (
    <svg width="20" height="20" viewBox="0 0 20 20" fill="none" aria-hidden="true">
      <rect x="1" y="1" width="18" height="18" rx="3" stroke="#E8622C" strokeWidth="2" />
      <path d="M1 7h18M7 1v18" stroke="#E8622C" strokeWidth="2" />
    </svg>
  );
}

function CrateStack() {
  return (
    <svg width="220" height="140" viewBox="0 0 220 140" fill="none" aria-hidden="true" className="opacity-90">
      {[
        { x: 10, y: 70, w: 70, h: 60 },
        { x: 86, y: 40, w: 70, h: 90 },
        { x: 162, y: 90, w: 50, h: 40 },
      ].map((box, i) => (
        <g key={i}>
          <rect x={box.x} y={box.y} width={box.w} height={box.h} rx="4" stroke={i === 1 ? "#E8622C" : "#EDEAE3"} strokeOpacity={i === 1 ? 1 : 0.35} strokeWidth="2" />
          <path
            d={`M${box.x} ${box.y + box.h * 0.4} H${box.x + box.w}`}
            stroke={i === 1 ? "#E8622C" : "#EDEAE3"}
            strokeOpacity={i === 1 ? 1 : 0.35}
            strokeWidth="2"
          />
        </g>
      ))}
    </svg>
  );
}
