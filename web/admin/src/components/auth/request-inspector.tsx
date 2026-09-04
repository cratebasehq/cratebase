import type { ApiFailure } from "@/lib/api";
import { cn } from "@/lib/utils";

/**
 * The live request inspector that sits beside the login form.
 *
 * This is not decoration. It shows the exact call the form is about to make,
 * then the exact envelope that came back — so a failed sign-in tells you
 * whether the server rejected your password, rejected the request, or never
 * answered at all. It is the brand moment for a backend product *and* the
 * diagnostic the old login was missing.
 */

export type AuthPhase =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "failed"; failure: ApiFailure; ms: number }
  | { kind: "succeeded"; token: string; id: string; email: string; ms: number };

export type Reachability = "checking" | "up" | "down";

const ENDPOINT = "/api/collections/_superusers/auth-with-password";

export function RequestInspector({
  identity,
  passwordLength,
  phase,
  className,
}: {
  identity: string;
  passwordLength: number;
  phase: AuthPhase;
  className?: string;
}) {
  return (
    <div className={cn("flex w-full max-w-measure flex-col gap-5 font-mono text-sm", className)}>
      <section aria-label="Request">
        <Line>
          <span className="text-primary">POST</span>{" "}
          <span className="text-foreground">{ENDPOINT}</span>
        </Line>
        <Line className="text-muted-foreground">content-type: application/json</Line>
        <div className="mt-3 text-muted-foreground">
          <Line>{"{"}</Line>
          <Line className="pl-4">
            <Key>identity</Key>
            {": "}
            <Str empty={identity.length === 0}>{identity || "…"}</Str>
            {","}
          </Line>
          <Line className="pl-4">
            <Key>password</Key>
            {": "}
            <Str empty={passwordLength === 0}>
              {passwordLength === 0 ? "…" : "•".repeat(Math.min(passwordLength, 24))}
            </Str>
          </Line>
          <Line>{"}"}</Line>
        </div>
      </section>

      <section
        aria-label="Response"
        aria-live="polite"
        className="min-h-28 border-t border-border pt-5"
      >
        <Response phase={phase} />
      </section>
    </div>
  );
}

/** The reachability read-out. Lives at the foot of the inspector panel so it
 * is answering "is the server up?" before anyone has typed anything. */
export function ServerStatusLine({
  reachability,
  origin,
  className,
}: {
  reachability: Reachability;
  origin: string;
  className?: string;
}) {
  return (
    <p className={cn("flex items-center gap-2 font-mono text-sm text-muted-foreground", className)}>
      <Dot state={reachability} />
      <span>
        {reachability === "checking"
          ? `contacting ${origin}`
          : reachability === "up"
            ? `${origin} · API is healthy`
            : `${origin} · no response`}
      </span>
    </p>
  );
}

function Response({ phase }: { phase: AuthPhase }) {
  if (phase.kind === "idle") {
    return (
      <p className="text-muted-foreground/70">
        <span className="text-muted-foreground">←</span> waiting for you
        <span className="ml-px inline-block motion-safe:animate-caret">▍</span>
      </p>
    );
  }

  if (phase.kind === "sending") {
    return (
      <p className="text-muted-foreground">
        <span>→</span> sending
        <span className="ml-px inline-block motion-safe:animate-caret">▍</span>
      </p>
    );
  }

  if (phase.kind === "succeeded") {
    return (
      <div className="text-muted-foreground">
        <Status code={200} label="OK" tone="success" ms={phase.ms} />
        <div className="mt-3">
          <Line>{"{"}</Line>
          <Line className="pl-4">
            <Key>token</Key>
            {": "}
            <Str>{`${phase.token.slice(0, 18)}…`}</Str>
            {","}
          </Line>
          <Line className="pl-4">
            <Key>record</Key>
            {": {"}
          </Line>
          <Line className="pl-8">
            <Key>id</Key>
            {": "}
            <Str>{phase.id}</Str>
            {","}
          </Line>
          <Line className="pl-8">
            <Key>email</Key>
            {": "}
            <Str>{phase.email}</Str>
          </Line>
          <Line className="pl-4">{"}"}</Line>
          <Line>{"}"}</Line>
        </div>
      </div>
    );
  }

  const { failure } = phase;
  const tone = failure.status === 0 ? "offline" : failure.status >= 500 ? "destructive" : "warning";

  return (
    <div className="text-muted-foreground">
      <Status
        code={failure.status}
        label={failure.status === 0 ? "no response" : statusText(failure.status)}
        tone={tone}
        ms={phase.ms}
      />
      <div className="mt-3">
        <Line>{"{"}</Line>
        <Line className="pl-4">
          <Key>status</Key>
          {": "}
          <span className="text-foreground">{failure.status}</span>
          {","}
        </Line>
        <Line className="pl-4">
          <Key>message</Key>
          {": "}
          {/* The server's own wording, not our rewritten copy — a panel that
              claims to show the envelope has to show the envelope. */}
          <Str>{failure.serverMessage || failure.title}</Str>
          {Object.keys(failure.fields).length > 0 ? "," : ""}
        </Line>
        {Object.keys(failure.fields).length > 0 ? (
          <>
            <Line className="pl-4">
              <Key>data</Key>
              {": {"}
            </Line>
            {Object.entries(failure.fields).map(([name, message]) => (
              <Line key={name} className="pl-8">
                <Key>{name}</Key>
                {": "}
                <Str>{message}</Str>
              </Line>
            ))}
            <Line className="pl-4">{"}"}</Line>
          </>
        ) : null}
        <Line>{"}"}</Line>
      </div>
    </div>
  );
}

function Status({
  code,
  label,
  tone,
  ms,
}: {
  code: number;
  label: string;
  tone: "success" | "warning" | "destructive" | "offline";
  ms: number;
}) {
  return (
    <p className="flex items-baseline gap-2">
      <span className="text-muted-foreground">←</span>
      <span
        className={cn(
          "font-medium",
          tone === "success" && "text-success",
          tone === "warning" && "text-warning",
          tone === "destructive" && "text-destructive",
          tone === "offline" && "text-destructive",
        )}
      >
        {code === 0 ? "—" : code} {label}
      </span>
      <span className="ml-auto font-tabular text-muted-foreground">{ms} ms</span>
    </p>
  );
}

function Line({ children, className }: { children?: React.ReactNode; className?: string }) {
  return <p className={cn("leading-5 break-words whitespace-pre-wrap", className)}>{children}</p>;
}

function Key({ children }: { children: React.ReactNode }) {
  return <span className="text-muted-foreground">"{children}"</span>;
}

function Str({ children, empty = false }: { children: React.ReactNode; empty?: boolean }) {
  return (
    <span className={empty ? "text-muted-foreground/50" : "text-foreground"}>"{children}"</span>
  );
}

function Dot({ state }: { state: Reachability }) {
  return (
    <span
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        state === "checking" && "bg-muted-foreground motion-safe:animate-pulse",
        state === "up" && "bg-success",
        state === "down" && "bg-destructive",
      )}
    />
  );
}

function statusText(code: number): string {
  switch (code) {
    case 400:
      return "Bad Request";
    case 401:
      return "Unauthorized";
    case 403:
      return "Forbidden";
    case 404:
      return "Not Found";
    case 429:
      return "Too Many Requests";
    case 500:
      return "Internal Server Error";
    default:
      return code >= 500 ? "Server Error" : "Error";
  }
}
