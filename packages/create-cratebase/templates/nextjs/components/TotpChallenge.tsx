"use client";

import { useState, type FormEvent } from "react";
import { cb } from "@/lib/cratebase";

/**
 * Shown when any first factor (password, OTP, or magic link) comes back
 * `401 { mfaId }` because the account also has TOTP confirmed — see
 * https://cratebase.dev/docs/concepts/authentication/ (MFA section) and
 * `AuthNamespace.signIn.totp` in `@cratebase/client`.
 */
export function TotpChallenge({ mfaId, onSuccess }: { mfaId: string; onSuccess: () => void }) {
  const [code, setCode] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setPending(true);
    setError(null);
    try {
      await cb.auth.signIn.totp({ mfaId, code });
      onSuccess();
    } catch (err) {
      setError(err instanceof Error ? err.message : "That code didn't work.");
    } finally {
      setPending(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-3">
      <p className="text-sm text-black/60 dark:text-white/60">
        Enter the 6-digit code from your authenticator app, or one of your backup codes.
      </p>
      <input
        className="input"
        inputMode="numeric"
        autoFocus
        placeholder="123456"
        value={code}
        onChange={(e) => setCode(e.target.value)}
      />
      {error && <p className="text-sm text-red-500">{error}</p>}
      <button type="submit" className="btn-primary" disabled={pending || code.length === 0}>
        {pending ? "Verifying…" : "Verify"}
      </button>
    </form>
  );
}
