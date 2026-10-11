import { useMemo, useState, type FormEvent } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowRight, CircleCheck, CircleX, Inbox, Send, Server } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { cb, describeFailure } from "@/lib/api";
import { useDevMailInboxAvailable, useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { StatusBadge } from "@/components/layout/stat-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import {
  NumberSetting,
  SecretSetting,
  SettingRow,
  SettingsPage,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

type Draft = Pick<ServerSettings, "smtp"> & { sender: { name: string; address: string } };

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function draftOf(settings: ServerSettings): Draft {
  return {
    smtp: { ...settings.smtp, password: "" },
    sender: { name: settings.meta.senderName, address: settings.meta.senderAddress },
  };
}

/** Only send a secret when one was typed — an empty box means "keep what
 * is stored", not "clear it". Sender identity lives in `meta`, which the
 * server expects whole, so the stored `meta` is carried along. */
function payloadOf(draft: Draft, settings: ServerSettings) {
  const smtp: Record<string, unknown> = { ...draft.smtp };
  if (!draft.smtp.password) delete smtp.password;
  return {
    smtp,
    meta: { ...settings.meta, senderName: draft.sender.name, senderAddress: draft.sender.address },
  };
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

/** Per-field messages, keyed by field. SMTP fields only matter while SMTP
 * is the selected backend. */
function fieldErrorsOf(draft: Draft): Record<string, string> {
  const errors: Record<string, string> = {};
  if (draft.sender.address.trim() && !EMAIL_RE.test(draft.sender.address.trim())) {
    errors.senderAddress = "Enter a valid email address, like no-reply@example.com";
  }
  if (draft.smtp.enabled) {
    if (!draft.smtp.host.trim()) errors.host = "SMTP needs a host, like smtp.example.com";
    if (!Number.isInteger(draft.smtp.port) || draft.smtp.port < 1 || draft.smtp.port > 65535) {
      errors.port = "Port must be a whole number between 1 and 65535";
    }
  }
  return errors;
}

interface TestResult {
  ok: boolean;
  message: string;
}

/** `Email → Delivery`: which backend sends outgoing mail, who it is sent
 * as, and the SMTP connection. S3-compatible file storage and the
 * image-transform/quota limits that used to live in this same "Delivery"
 * tab moved to `Application → Storage` — they're a storage concern, not
 * an email one (see `ApplicationPage`). */
export function EmailDeliveryPage() {
  const { data: settings, isPending } = useSettings();
  const { data: devMailInboxAvailable } = useDevMailInboxAvailable();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);
  const [testAddress, setTestAddress] = useState("");
  const [testTouched, setTestTouched] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const sendTestEmail = useMutation({
    mutationFn: (email: string) => cb.admin.settings.testEmail("_superusers", email, "verification"),
    onSuccess: (_result, email) => {
      const message = settings?.smtp.enabled
        ? `Sent to ${email} through ${settings.smtp.host || "SMTP"}.`
        : `Captured in the dev inbox for ${email} — nothing left this server.`;
      setTestResult({ ok: true, message });
      toast.success(`Test email sent to ${email}`);
    },
    onError: (error) => {
      const failure = describeFailure(error);
      const reason = Object.values(failure.fields)[0] ?? failure.serverMessage ?? failure.detail;
      setTestResult({ ok: false, message: reason || "The server didn't say why." });
      toast.error("The test email failed", { description: reason });
    },
  });

  const fieldErrors = useMemo(() => (draft ? fieldErrorsOf(draft) : {}), [draft]);
  const errors = Object.values(fieldErrors);

  const item = settingsItemFor("/settings/mail-storage")!;

  if (isPending || !draft || !settings) {
    return (
      <SettingsPage title={item.label} description={item.description} width="form">
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-40 w-full" />
        <Skeleton className="h-64 w-full" />
      </SettingsPage>
    );
  }

  const dirty = isDirty(draft, settings);
  const smtp = draft.smtp;
  const testAddressInvalid = !EMAIL_RE.test(testAddress.trim());
  const showTestError = testTouched && testAddress.trim() !== "" && testAddressInvalid;

  function submit() {
    if (!draft || !settings || errors.length > 0) return;
    save.mutate(payloadOf(draft, settings), {
      onSuccess: () => {
        toast.success("Email delivery settings saved");
        // The secret was consumed; clear the box so it reads as "stored".
        setDraft((d) => (d ? { ...d, smtp: { ...d.smtp, password: "" } } : d));
      },
      onError: (error) => {
        const failure = describeFailure(error);
        const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
        toast.error("Couldn't save the delivery settings", {
          description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
        });
      },
    });
  }

  function onTestSubmit(event: FormEvent) {
    event.preventDefault();
    setTestTouched(true);
    if (testAddressInvalid || sendTestEmail.isPending) return;
    setTestResult(null);
    sendTestEmail.mutate(testAddress.trim());
  }

  const backends = [
    {
      enabled: true,
      icon: <Server />,
      title: "SMTP server",
      body: "Deliver mail through your own SMTP provider. Use this in production.",
    },
    {
      enabled: false,
      icon: <Inbox />,
      title: "Log (development)",
      body: "Nothing is sent. Mail is captured in the dev inbox so you can read verification and reset links.",
    },
  ];

  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      <SettingsSection
        title="Delivery backend"
        description="Where verification, password-reset, OTP and template emails go."
        action={
          settings.smtp.enabled ? (
            <StatusBadge tone="success">Active: SMTP</StatusBadge>
          ) : (
            <StatusBadge tone="warning">Active: Log (not delivered)</StatusBadge>
          )
        }
      >
        <div role="group" aria-label="Delivery backend" className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {backends.map((backend) => {
            const selected = smtp.enabled === backend.enabled;
            return (
              <button
                key={backend.title}
                type="button"
                aria-pressed={selected}
                onClick={() => setDraft({ ...draft, smtp: { ...smtp, enabled: backend.enabled } })}
                className={cn(
                  "flex flex-col items-start gap-1 rounded-lg border p-3 text-left transition-colors duration-fast ease-out-strong focus-visible:ring-3 focus-visible:ring-ring/40",
                  selected
                    ? "border-primary bg-brand-soft"
                    : "border-border bg-card hover:border-border-strong hover:bg-muted/50",
                )}
              >
                <span className="flex items-center gap-1.5 text-sm font-medium [&_svg]:size-4 [&_svg]:shrink-0">
                  {backend.icon}
                  {backend.title}
                </span>
                <span className="text-xs text-pretty text-muted-foreground">{backend.body}</span>
              </button>
            );
          })}
        </div>
        {!smtp.enabled && devMailInboxAvailable ? (
          <p className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
            Mail is currently held in memory.
            <Link
              to="/settings/email"
              search={{ tab: "dev-inbox" }}
              className="inline-flex items-center gap-1 text-brand-text hover:underline"
            >
              Open dev inbox <ArrowRight className="size-3" />
            </Link>
          </p>
        ) : null}
      </SettingsSection>

      <SettingsSection
        title="Sender identity"
        description="The From name and address recipients see. Also editable under Application."
      >
        <SettingRow label="Sender name" htmlFor="sender-name" help="Shown as the display name, e.g. “Acme Support”.">
          <TextSetting
            id="sender-name"
            value={draft.sender.name}
            onChange={(name) => setDraft({ ...draft, sender: { ...draft.sender, name } })}
            placeholder="Support"
          />
        </SettingRow>
        <SettingRow
          label="Sender address"
          htmlFor="sender-address"
          help="Use an address on a domain your SMTP provider is allowed to send for, or mail may be rejected."
          error={fieldErrors.senderAddress}
        >
          <TextSetting
            id="sender-address"
            type="email"
            mono
            value={draft.sender.address}
            onChange={(address) => setDraft({ ...draft, sender: { ...draft.sender, address } })}
            placeholder="no-reply@example.com"
            invalid={Boolean(fieldErrors.senderAddress)}
          />
        </SettingRow>
      </SettingsSection>

      {smtp.enabled ? (
        <SettingsSection
          title="SMTP connection"
          description="Credentials are stored on the server and never sent back to this page."
        >
          <SettingRow label="Host" htmlFor="smtp-host" error={fieldErrors.host}>
            <TextSetting
              id="smtp-host"
              value={smtp.host}
              onChange={(host) => setDraft({ ...draft, smtp: { ...smtp, host } })}
              placeholder="smtp.example.com"
              mono
              invalid={Boolean(fieldErrors.host)}
            />
          </SettingRow>
          <SettingRow
            label="Port"
            htmlFor="smtp-port"
            help="Usually 587 with STARTTLS, or 465 with implicit TLS."
            error={fieldErrors.port}
          >
            <NumberSetting
              id="smtp-port"
              min={1}
              value={smtp.port}
              onChange={(port) => setDraft({ ...draft, smtp: { ...smtp, port } })}
              invalid={Boolean(fieldErrors.port)}
            />
          </SettingRow>
          <SettingRow label="Username" htmlFor="smtp-user" help="Leave empty if the server needs no authentication.">
            <TextSetting
              id="smtp-user"
              value={smtp.username}
              onChange={(username) => setDraft({ ...draft, smtp: { ...smtp, username } })}
              mono
            />
          </SettingRow>
          <SettingRow
            label="Password"
            htmlFor="smtp-pass"
            help="Write-only. Leave blank to keep the stored password; type a new one to replace it."
          >
            <SecretSetting
              id="smtp-pass"
              value={smtp.password ?? ""}
              onChange={(password) => setDraft({ ...draft, smtp: { ...smtp, password } })}
              storedHint="•••••••• (stored — unchanged)"
            />
          </SettingRow>
          <SettingRow
            label="Encryption"
            htmlFor="smtp-tls"
            help="Implicit TLS connects encrypted from the first byte (port 465). STARTTLS upgrades a plain connection (port 587)."
          >
            <ToggleSetting
              id="smtp-tls"
              checked={smtp.tls}
              onChange={(tls) => setDraft({ ...draft, smtp: { ...smtp, tls } })}
              label={smtp.tls ? "Implicit TLS" : "STARTTLS"}
            />
          </SettingRow>
        </SettingsSection>
      ) : null}

      <SettingsSection
        title="Send a test email"
        description={
          settings.smtp.enabled
            ? "Sends a sample verification email through the saved SMTP settings."
            : "Sends a sample verification email to the dev inbox — the Log backend never delivers."
        }
      >
        <form onSubmit={onTestSubmit} noValidate className="flex flex-col gap-2">
          <div className="flex flex-wrap items-start gap-2">
            <div className="flex min-w-0 flex-col gap-1">
              <label htmlFor="test-email" className="sr-only">
                Test recipient
              </label>
              <Input
                id="test-email"
                type="email"
                value={testAddress}
                onChange={(e) => {
                  setTestAddress(e.target.value);
                  setTestResult(null);
                }}
                onBlur={() => setTestTouched(true)}
                placeholder="you@example.com"
                aria-invalid={showTestError || undefined}
                aria-describedby={showTestError ? "test-email-error" : undefined}
                className="h-control-md w-64 max-w-full"
              />
              {showTestError ? (
                <p id="test-email-error" role="alert" className="text-xs text-destructive">
                  Enter a valid email address.
                </p>
              ) : null}
            </div>
            <Button type="submit" variant="outline" disabled={sendTestEmail.isPending || testAddress.trim() === ""}>
              {sendTestEmail.isPending ? <Spinner /> : <Send />}
              Send test email
            </Button>
          </div>
          {dirty ? (
            <p className="text-xs text-warning">You have unsaved changes — the test uses the saved settings. Save first.</p>
          ) : null}
          {testResult ? (
            <p
              role={testResult.ok ? "status" : "alert"}
              className={cn(
                "flex items-start gap-1.5 text-xs [&_svg]:mt-px [&_svg]:size-3.5 [&_svg]:shrink-0",
                testResult.ok ? "text-success" : "text-destructive",
              )}
            >
              {testResult.ok ? <CircleCheck /> : <CircleX />}
              <span className="break-words">{testResult.message}</span>
            </p>
          ) : null}
        </form>
      </SettingsSection>

      <SettingsSaveBar
        dirty={dirty}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </SettingsPage>
  );
}
