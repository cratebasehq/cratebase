import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowRight, Inbox, Send } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { cb, describeFailure } from "@/lib/api";
import { useDevMailInboxAvailable, useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { settingsItemFor } from "@/lib/settings-nav";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
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

type Draft = Pick<ServerSettings, "smtp">;

function draftOf(settings: ServerSettings): Draft {
  return {
    smtp: { ...settings.smtp, password: "" },
  };
}

/** Only send a secret when one was typed — an empty box means "keep what
 * is stored", not "clear it". */
function payloadOf(draft: Draft) {
  const smtp: Record<string, unknown> = { ...draft.smtp };
  if (!draft.smtp.password) delete smtp.password;
  return { smtp };
}

function validate(draft: Draft): string[] {
  const errors: string[] = [];
  if (draft.smtp.enabled) {
    if (!draft.smtp.host.trim()) errors.push("SMTP needs a host");
    if (draft.smtp.port < 1 || draft.smtp.port > 65535) errors.push("SMTP port must be between 1 and 65535");
  }
  return errors;
}

/** `Email → Delivery`: SMTP for outgoing mail. S3-compatible file storage
 * and the image-transform/quota limits that used to live in this same
 * "Delivery" tab moved to `Application → Storage` — they're a storage
 * concern, not an email one, and being tucked in here made them easy to
 * miss (see `ApplicationPage`). */
export function EmailDeliveryPage() {
  const { data: settings, isPending } = useSettings();
  const { data: devMailInboxAvailable } = useDevMailInboxAvailable();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);
  const [testAddress, setTestAddress] = useState("");

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const sendTestEmail = useMutation({
    mutationFn: (email: string) => cb.admin.settings.testEmail("_superusers", email, "verification"),
    onSuccess: () => toast.success("Test email sent", { description: "Check the inbox, and the request logs." }),
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("The test email failed", {
        description: Object.values(failure.fields)[0] ?? failure.serverMessage ?? failure.detail,
      });
    },
  });

  const errors = useMemo(() => (draft ? validate(draft) : []), [draft]);

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Settings saved");
        // The secret was consumed; clear the box so it reads as "stored".
        setDraft((d) => (d ? { ...d, smtp: { ...d.smtp, password: "" } } : d));
      },
      onError: (error) => {
        const failure = describeFailure(error);
        const fields = Object.entries(failure.fields).map(([k, v]) => `${k}: ${v}`);
        toast.error(failure.title, {
          description: fields.length > 0 ? fields.join("; ") : failure.serverMessage || failure.detail,
        });
      },
    });
  }

  const item = settingsItemFor("/settings/mail-storage")!;
  return (
    <SettingsPage title={item.label} description={item.description} width="form">
      <SettingsSection
        title="SMTP"
        description="Where verification, password-reset and OTP emails are sent from. Off means the server only logs them."
      >
        {devMailInboxAvailable ? (
          <Alert>
            <Inbox />
            <AlertTitle>SMTP is off — mail is going to the dev inbox</AlertTitle>
            <AlertDescription>
              Every email sent while SMTP is disabled lands in memory instead, including verification /
              password-reset / OTP links.{" "}
              <Link to="/settings/email" search={{ tab: "dev-inbox" }} className="inline-flex items-center gap-1">
                Open Dev inbox <ArrowRight className="size-3.5" />
              </Link>
            </AlertDescription>
          </Alert>
        ) : null}
        <SettingRow label="Enabled" htmlFor="smtp-enabled">
          <ToggleSetting
            id="smtp-enabled"
            checked={draft.smtp.enabled}
            onChange={(enabled) => setDraft({ ...draft, smtp: { ...draft.smtp, enabled } })}
            label={draft.smtp.enabled ? "Sending through SMTP" : "Not sending mail"}
          />
        </SettingRow>
        <SettingRow label="Host" htmlFor="smtp-host">
          <TextSetting
            id="smtp-host"
            value={draft.smtp.host}
            onChange={(host) => setDraft({ ...draft, smtp: { ...draft.smtp, host } })}
            placeholder="smtp.example.com"
            mono
          />
        </SettingRow>
        <SettingRow label="Port" htmlFor="smtp-port">
          <NumberSetting
            id="smtp-port"
            min={1}
            value={draft.smtp.port}
            onChange={(port) => setDraft({ ...draft, smtp: { ...draft.smtp, port } })}
          />
        </SettingRow>
        <SettingRow label="Username" htmlFor="smtp-user">
          <TextSetting
            id="smtp-user"
            value={draft.smtp.username}
            onChange={(username) => setDraft({ ...draft, smtp: { ...draft.smtp, username } })}
          />
        </SettingRow>
        <SettingRow
          label="Password"
          htmlFor="smtp-pass"
          help="Never sent back by the server. Leave blank to keep the stored one."
        >
          <SecretSetting
            id="smtp-pass"
            value={draft.smtp.password ?? ""}
            onChange={(password) => setDraft({ ...draft, smtp: { ...draft.smtp, password } })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow label="TLS" htmlFor="smtp-tls" help="Connect over TLS immediately rather than upgrading with STARTTLS.">
          <ToggleSetting
            id="smtp-tls"
            checked={draft.smtp.tls}
            onChange={(tls) => setDraft({ ...draft, smtp: { ...draft.smtp, tls } })}
            label={draft.smtp.tls ? "Implicit TLS" : "STARTTLS"}
          />
        </SettingRow>
        <SettingRow
          label="Send a test"
          help="Uses the currently saved settings — save first if you have just changed them."
        >
          <div className="flex flex-wrap items-center gap-2">
            <Input
              type="email"
              value={testAddress}
              onChange={(e) => setTestAddress(e.target.value)}
              placeholder="you@example.com"
              className="h-control-md w-56"
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-control-md gap-1.5"
              disabled={sendTestEmail.isPending || testAddress.trim().length === 0}
              onClick={() => sendTestEmail.mutate(testAddress.trim())}
            >
              {sendTestEmail.isPending ? <Spinner /> : <Send className="size-3.5" />}
              Send test
            </Button>
          </div>
        </SettingRow>
      </SettingsSection>

      <SettingsSaveBar
        dirty={isDirty(draft, settings)}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </SettingsPage>
  );
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}
