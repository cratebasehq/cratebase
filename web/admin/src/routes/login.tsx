import { useEffect, useRef, useState } from "react";
import { CratebaseError } from "@cratebase/client";
import { createRoute, redirect, useNavigate } from "@tanstack/react-router";
import { Eye, EyeOff, TriangleAlert } from "lucide-react";
import { CratebaseMark } from "@/components/brand/cratebase-mark";
import {
  RequestInspector,
  ServerStatusLine,
  type AuthPhase,
  type Reachability,
} from "@/components/auth/request-inspector";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import {
  authWithPassword,
  checkHealth,
  checkSetupStatus,
  createFirstSuperuser,
  describeFailure,
  isLoggedIn,
} from "@/lib/api";
import { rootRoute } from "@/routes/root";

function LoginPage() {
  const navigate = useNavigate();

  const [identity, setIdentity] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [capsLock, setCapsLock] = useState(false);
  const [phase, setPhase] = useState<AuthPhase>({ kind: "idle" });
  const [reachability, setReachability] = useState<Reachability>("checking");
  const [clientErrors, setClientErrors] = useState<Record<string, string>>({});
  const identityRef = useRef<HTMLInputElement>(null);
  const passwordRef = useRef<HTMLInputElement>(null);
  // "checking": neither form renders yet, avoiding a login-form flash
  // before the answer arrives. "needed": no superuser exists — render the
  // first-run setup form instead of login. "resolved": normal login.
  const [setupStatus, setSetupStatus] = useState<"checking" | "needed" | "resolved">("checking");

  // Ask the server whether it is even there before anyone types a password.
  // `/api/health` is unauthenticated, so this is the honest answer to the
  // failure the old login reported as "Invalid email or password."
  useEffect(() => {
    let cancelled = false;
    checkHealth().then(
      () => !cancelled && setReachability("up"),
      () => !cancelled && setReachability("down"),
    );
    return () => {
      cancelled = true;
    };
  }, []);

  // A fresh database has no superuser and therefore no way to log in —
  // ask before rendering either form. A failed check (server down, old
  // Cratebase without this endpoint) falls back to the ordinary login
  // form rather than getting stuck on "checking" forever.
  useEffect(() => {
    let cancelled = false;
    checkSetupStatus().then(
      (result) => !cancelled && setSetupStatus(result.needsSetup ? "needed" : "resolved"),
      () => !cancelled && setSetupStatus("resolved"),
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const pending = phase.kind === "sending";
  const failure = phase.kind === "failed" ? phase.failure : null;
  const identityError =
    clientErrors["identity"] ?? failure?.fields["identity"] ?? failure?.fields["email"];
  const passwordError = clientErrors["password"] ?? failure?.fields["password"];

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;

    // The submit button stays enabled when fields are empty: a disabled
    // button that never says why is the same dead end as the old form that
    // swallowed Enter. Say what is missing instead.
    const problems: Record<string, string> = {};
    if (identity.trim().length === 0) problems["identity"] = "Enter your email address.";
    else if (!identity.includes("@")) problems["identity"] = "That doesn't look like an email address.";
    if (password.length === 0) problems["password"] = "Enter your password.";
    if (Object.keys(problems).length > 0) {
      setClientErrors(problems);
      (problems["identity"] ? identityRef : passwordRef).current?.focus();
      return;
    }
    setClientErrors({});

    setPhase({ kind: "sending" });
    const startedAt = performance.now();
    try {
      const result = await authWithPassword(identity.trim(), password);
      setPhase({
        kind: "succeeded",
        token: result.token,
        id: result.record.id,
        email: typeof result.record.email === "string" ? result.record.email : "",
        ms: Math.round(performance.now() - startedAt),
      });
      setReachability("up");
      await navigate({ to: "/" });
    } catch (error) {
      const described = describeFailure(error, "auth");
      setPhase({
        kind: "failed",
        failure: described,
        ms: Math.round(performance.now() - startedAt),
      });
      if (described.offline) setReachability("down");
      // Focus follows the error: a rejected sign-in is almost always the
      // password, so put the cursor where the fix is.
      if (!described.offline) passwordRef.current?.select();
    }
  }

  function trackCapsLock(event: React.KeyboardEvent<HTMLInputElement>) {
    setCapsLock(event.getModifierState("CapsLock"));
  }

  return (
    <div className="grid min-h-svh lg:grid-cols-2">
      {/* --- Form column -------------------------------------------------
          Brand, form and footnote all hang off one left axis; the column as
          a whole is centred in its half. Nothing is boxed — a card here
          would just be a border drawn around the only thing on screen. */}
      <div className="mx-auto flex w-full max-w-[21rem] flex-col px-6 py-page sm:px-0">
        <header>
          <span className="flex items-center gap-2">
            <CratebaseMark className="size-5" animated />
            <span className="text-sm font-medium tracking-tight">Cratebase</span>
          </span>
        </header>

        <main className="flex flex-1 items-center py-12">
          <div className="w-full">
            {setupStatus === "checking" ? null : setupStatus === "needed" ? (
              <FirstRunSetupForm onFallbackToLogin={() => setSetupStatus("resolved")} />
            ) : (
              <>
                <h1 className="text-3xl font-medium tracking-tight">Sign in</h1>
                <p className="mt-2 text-sm text-muted-foreground">
                  Superuser access to{" "}
                  <span className="font-mono text-foreground">{displayOrigin()}</span>
                </p>

                <form onSubmit={handleSubmit} className="mt-8 flex flex-col gap-4" noValidate>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="identity" className="text-xs">
                  Email
                </Label>
                <Input
                  id="identity"
                  ref={identityRef}
                  name="identity"
                  type="email"
                  inputMode="email"
                  value={identity}
                  onChange={(event) => setIdentity(event.target.value)}
                  autoComplete="username"
                  autoFocus
                  required
                  disabled={pending}
                  aria-invalid={identityError ? true : undefined}
                  aria-describedby={identityError ? "identity-error" : undefined}
                  placeholder="you@example.com"
                  className="h-control-lg"
                />
                {identityError ? (
                  <p id="identity-error" className="text-xs text-destructive">
                    {identityError}
                  </p>
                ) : null}
              </div>

              <div className="flex flex-col gap-1.5">
                <div className="flex items-baseline justify-between gap-2">
                  <Label htmlFor="password" className="text-xs">
                    Password
                  </Label>
                  {capsLock ? (
                    <span className="flex items-center gap-1 text-xs text-warning">
                      <TriangleAlert className="size-3" />
                      Caps lock is on
                    </span>
                  ) : null}
                </div>
                <InputGroup className="h-control-lg">
                  <InputGroupInput
                    id="password"
                    ref={passwordRef}
                    name="password"
                    type={showPassword ? "text" : "password"}
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    onKeyDown={trackCapsLock}
                    onKeyUp={trackCapsLock}
                    onBlur={() => setCapsLock(false)}
                    autoComplete="current-password"
                    required
                    disabled={pending}
                    aria-invalid={passwordError ? true : undefined}
                    aria-describedby={passwordError ? "password-error" : undefined}
                  />
                  <InputGroupAddon align="inline-end">
                    <InputGroupButton
                      type="button"
                      onClick={() => setShowPassword((shown) => !shown)}
                      aria-label={showPassword ? "Hide password" : "Show password"}
                      aria-pressed={showPassword}
                    >
                      {showPassword ? <EyeOff /> : <Eye />}
                    </InputGroupButton>
                  </InputGroupAddon>
                </InputGroup>
                {passwordError ? (
                  <p id="password-error" className="text-xs text-destructive">
                    {passwordError}
                  </p>
                ) : null}
              </div>

              {failure && !identityError && !passwordError ? (
                <Alert
                  variant="destructive"
                  role="alert"
                  className="border-destructive/25 bg-destructive/5"
                >
                  <TriangleAlert />
                  <AlertTitle>{failure.title}</AlertTitle>
                  {failure.detail ? <AlertDescription>{failure.detail}</AlertDescription> : null}
                </Alert>
              ) : null}

              <Button
                type="submit"
                size="lg"
                disabled={pending}
                className="mt-1 w-full shadow-lift"
              >
                {pending ? <Spinner /> : null}
                {pending ? "Signing in…" : "Sign in"}
              </Button>

              {/* Room for the alternate methods the server already supports —
                  OAuth2 providers, one-time codes, and the MFA second step
                  all land here, above the CLI hint, once the settings screen
                  can enable them. Nothing renders while none are configured. */}
              <AlternateAuthMethods />
            </form>
              </>
            )}
          </div>
        </main>
      </div>

      {/* --- Inspector column --------------------------------------------
          Hidden below lg: every failure the panel explains is also stated in
          the alert above, so nothing is only here. */}
      <aside className="hidden items-center border-l border-border bg-surface-sunken px-10 py-page lg:flex">
        <div className="flex w-full max-w-measure flex-col gap-5">
          <RequestInspector
            identity={identity.trim()}
            passwordLength={password.length}
            phase={phase}
          />
          <ServerStatusLine
            reachability={reachability}
            origin={displayOrigin()}
            className="border-t border-border pt-4"
          />
        </div>
      </aside>
    </div>
  );
}

/** Placeholder for OAuth2 / OTP / MFA entry points. Deliberately renders
 * nothing: a row of dead "Continue with…" buttons would be a lie. */
function AlternateAuthMethods() {
  return null;
}

/** Rendered instead of the login form when `GET /api/setup/status` says no
 * superuser exists yet. Creates the first superuser, then signs in with the
 * same credentials exactly like a normal login — `POST /api/setup` never
 * mints a token itself. */
function FirstRunSetupForm({ onFallbackToLogin }: { onFallbackToLogin: () => void }) {
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [passwordConfirm, setPasswordConfirm] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [pending, setPending] = useState(false);
  const [fields, setFields] = useState<Record<string, string>>({});
  const [notice, setNotice] = useState<string | null>(null);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;

    const problems: Record<string, string> = {};
    if (!email.includes("@")) problems["email"] = "That doesn't look like an email address.";
    if (password.length < 8) problems["password"] = "At least 8 characters.";
    if (password !== passwordConfirm) problems["passwordConfirm"] = "Values don't match.";
    if (Object.keys(problems).length > 0) {
      setFields(problems);
      return;
    }
    setFields({});
    setPending(true);
    try {
      await createFirstSuperuser(email.trim(), password, passwordConfirm);
      try {
        await authWithPassword(email.trim(), password);
        await navigate({ to: "/" });
        return;
      } catch {
        // Created but the immediate login somehow failed — fall back to
        // the normal form rather than get stuck on a dead setup screen.
        onFallbackToLogin();
        return;
      }
    } catch (error) {
      const described = describeFailure(error, "generic");
      if (error instanceof CratebaseError && error.status === 403) {
        // Someone else finished setup in the gap between this page loading
        // and this submit — re-derive from a fresh read, not the stale
        // "needed" state that got us here.
        setNotice("Someone else just finished setup. Sign in below.");
        onFallbackToLogin();
        return;
      }
      setFields(described.fields);
      setNotice(described.detail || described.title);
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      <h1 className="text-3xl font-medium tracking-tight">Create your first superuser</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        This Cratebase instance has no superuser yet — create one to open the dashboard.
      </p>

      <form onSubmit={handleSubmit} className="mt-8 flex flex-col gap-4" noValidate>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="setup-email" className="text-xs">
            Email
          </Label>
          <Input
            id="setup-email"
            type="email"
            inputMode="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            autoComplete="username"
            autoFocus
            required
            disabled={pending}
            aria-invalid={fields["email"] ? true : undefined}
            placeholder="you@example.com"
            className="h-control-lg"
          />
          {fields["email"] ? (
            <p className="text-xs text-destructive">{fields["email"]}</p>
          ) : null}
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="setup-password" className="text-xs">
            Password
          </Label>
          <InputGroup className="h-control-lg">
            <InputGroupInput
              id="setup-password"
              type={showPassword ? "text" : "password"}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              autoComplete="new-password"
              required
              disabled={pending}
              aria-invalid={fields["password"] ? true : undefined}
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                type="button"
                onClick={() => setShowPassword((shown) => !shown)}
                aria-label={showPassword ? "Hide password" : "Show password"}
                aria-pressed={showPassword}
              >
                {showPassword ? <EyeOff /> : <Eye />}
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
          {fields["password"] ? (
            <p className="text-xs text-destructive">{fields["password"]}</p>
          ) : null}
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="setup-password-confirm" className="text-xs">
            Confirm password
          </Label>
          <Input
            id="setup-password-confirm"
            type={showPassword ? "text" : "password"}
            value={passwordConfirm}
            onChange={(event) => setPasswordConfirm(event.target.value)}
            autoComplete="new-password"
            required
            disabled={pending}
            aria-invalid={fields["passwordConfirm"] ? true : undefined}
            className="h-control-lg"
          />
          {fields["passwordConfirm"] ? (
            <p className="text-xs text-destructive">{fields["passwordConfirm"]}</p>
          ) : null}
        </div>

        {notice ? (
          <Alert variant="destructive" role="alert" className="border-destructive/25 bg-destructive/5">
            <TriangleAlert />
            <AlertTitle>{notice}</AlertTitle>
          </Alert>
        ) : null}

        <Button type="submit" size="lg" disabled={pending} className="mt-1 w-full shadow-lift">
          {pending ? <Spinner /> : null}
          {pending ? "Creating…" : "Create superuser"}
        </Button>
      </form>
    </>
  );
}

function displayOrigin(): string {
  const configured = import.meta.env.VITE_API_URL;
  if (typeof configured === "string" && configured.length > 0) {
    return configured.replace(/^https?:\/\//, "");
  }
  return typeof window === "undefined" ? "this server" : window.location.host;
}

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  beforeLoad: () => {
    // A signed-in superuser landing on /login (a bookmark, a stale tab) goes
    // straight through rather than being asked to authenticate twice.
    if (isLoggedIn()) throw redirect({ to: "/" });
  },
  component: LoginPage,
});
