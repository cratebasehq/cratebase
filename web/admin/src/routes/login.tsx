import { useState } from "react";
import { createRoute, useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { InlineValidation } from "@/components/interior/inline-validation";
import { LoadingButton } from "@/components/interior/loading-button";
import { cb } from "@/lib/api";
import { rootRoute } from "@/routes/root";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function validateEmail(value: string): string | null {
  if (value.length === 0) return "Email is required";
  return EMAIL_RE.test(value) ? null : "Enter a valid email address";
}

function LoginPage() {
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  async function handleLogin() {
    if (!EMAIL_RE.test(email) || password.length === 0) {
      toast.error("Enter your email and password.");
      throw new Error("invalid form");
    }
    try {
      await cb.admins.authWithPassword(email, password);
      await navigate({ to: "/" });
    } catch {
      toast.error("Invalid email or password.");
      throw new Error("login failed");
    }
  }

  return (
    <div className="flex min-h-svh items-center justify-center bg-background px-4">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex flex-col items-center gap-2 text-center">
          <img src="/favicon.svg" alt="" className="size-10" />
          <div>
            <h1 className="text-lg font-semibold tracking-tight">Cratebase</h1>
            <p className="text-sm text-muted-foreground">Sign in to manage your backend</p>
          </div>
        </div>

        <form
          className="flex flex-col gap-3 rounded-2xl border border-border bg-card p-6 shadow-sm"
          onSubmit={(e) => e.preventDefault()}
        >
          <InlineValidation
            label="Email"
            type="email"
            value={email}
            onChange={setEmail}
            validate={validateEmail}
            placeholder="you@example.com"
            autoComplete="email"
            required
          />

          <div className="flex flex-col gap-1.5">
            <label htmlFor="password" className="text-[13px] font-medium text-foreground">
              Password
            </label>
            <input
              id="password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              required
              className="h-10 w-full rounded-[10px] border-2 border-border bg-secondary/60 px-3 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-primary focus:bg-card"
            />
          </div>

          <LoadingButton
            onAction={handleLogin}
            pendingLabel="Signing in…"
            successLabel="Signed in"
            errorLabel="Try again"
            className="!h-10 !w-full !justify-center !border-primary !bg-primary !text-[13px] !text-primary-foreground hover:!bg-primary/90 dark:!border-primary dark:!bg-primary dark:!text-primary-foreground dark:hover:!bg-primary/90"
          >
            Sign in
          </LoadingButton>
        </form>

        <p className="mt-6 text-center text-xs text-muted-foreground">
          No superuser yet? Run <code className="rounded bg-secondary px-1 py-0.5 font-mono">cratebase superuser create</code>.
        </p>
      </div>
    </div>
  );
}

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: LoginPage,
});
