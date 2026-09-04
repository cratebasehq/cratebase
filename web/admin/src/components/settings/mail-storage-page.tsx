import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { Send, TestTube } from "lucide-react";
import { cb, describeFailure } from "@/lib/api";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import {
  NumberSetting,
  SecretSetting,
  SettingRow,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

type Draft = Pick<ServerSettings, "smtp" | "s3">;

function draftOf(settings: ServerSettings): Draft {
  return {
    smtp: { ...settings.smtp, password: "" },
    s3: { ...settings.s3, secret: "" },
  };
}

/** Only send a secret when one was typed — an empty box means "keep what
 * is stored", not "clear it". */
function payloadOf(draft: Draft) {
  const smtp: Record<string, unknown> = { ...draft.smtp };
  if (!draft.smtp.password) delete smtp.password;
  const s3: Record<string, unknown> = { ...draft.s3 };
  if (!draft.s3.secret) delete s3.secret;
  return { smtp, s3 };
}

function validate(draft: Draft): string[] {
  const errors: string[] = [];
  if (draft.smtp.enabled) {
    if (!draft.smtp.host.trim()) errors.push("SMTP needs a host");
    if (draft.smtp.port < 1 || draft.smtp.port > 65535) errors.push("SMTP port must be between 1 and 65535");
  }
  if (draft.s3.enabled) {
    if (!draft.s3.bucket.trim()) errors.push("S3 needs a bucket");
    if (!draft.s3.endpoint.trim()) errors.push("S3 needs an endpoint");
  }
  return errors;
}

export function MailStoragePage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);
  const [testAddress, setTestAddress] = useState("");

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  const sendTestEmail = useMutation({
    mutationFn: (email: string) => cb.settings.testEmail("_superusers", email, "verification"),
    onSuccess: () => toast.success("Test email sent", { description: "Check the inbox, and the request logs." }),
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("The test email failed", {
        description: Object.values(failure.fields)[0] ?? failure.serverMessage ?? failure.detail,
      });
    },
  });

  const testS3 = useMutation({
    mutationFn: (which: "storage" | "backups") => cb.settings.testS3(which),
    onSuccess: () => toast.success("S3 reachable", { description: "The bucket answered." }),
    onError: (error) => {
      const failure = describeFailure(error);
      toast.error("S3 test failed", { description: failure.serverMessage || failure.detail });
    },
  });

  const errors = useMemo(() => (draft ? validate(draft) : []), [draft]);

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  function submit() {
    if (!draft || errors.length > 0) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Settings saved");
        // Secrets were consumed; clear the boxes so they read as "stored".
        setDraft((d) => (d ? { smtp: { ...d.smtp, password: "" }, s3: { ...d.s3, secret: "" } } : d));
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

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
      <SettingsSection
        title="SMTP"
        description="Where verification, password-reset and OTP emails are sent from. Off means the server only logs them."
      >
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

      <SettingsSection
        title="File storage"
        description="Where uploaded files live. Off keeps them on the server's own disk under the data directory."
        action={
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-control-sm shrink-0 gap-1.5"
            disabled={testS3.isPending}
            onClick={() => testS3.mutate("storage")}
          >
            {testS3.isPending ? <Spinner /> : <TestTube className="size-3.5" />}
            Test connection
          </Button>
        }
      >
        <SettingRow label="Use S3" htmlFor="s3-enabled">
          <ToggleSetting
            id="s3-enabled"
            checked={draft.s3.enabled}
            onChange={(enabled) => setDraft({ ...draft, s3: { ...draft.s3, enabled } })}
            label={draft.s3.enabled ? "Uploads go to S3" : "Uploads stay on local disk"}
          />
        </SettingRow>
        <SettingRow label="Bucket" htmlFor="s3-bucket">
          <TextSetting
            id="s3-bucket"
            value={draft.s3.bucket}
            onChange={(bucket) => setDraft({ ...draft, s3: { ...draft.s3, bucket } })}
            mono
          />
        </SettingRow>
        <SettingRow label="Region" htmlFor="s3-region">
          <TextSetting
            id="s3-region"
            value={draft.s3.region}
            onChange={(region) => setDraft({ ...draft, s3: { ...draft.s3, region } })}
            placeholder="us-east-1"
            mono
          />
        </SettingRow>
        <SettingRow label="Endpoint" htmlFor="s3-endpoint" help="Any S3-compatible endpoint — R2, MinIO, Spaces.">
          <TextSetting
            id="s3-endpoint"
            value={draft.s3.endpoint}
            onChange={(endpoint) => setDraft({ ...draft, s3: { ...draft.s3, endpoint } })}
            placeholder="https://s3.amazonaws.com"
            mono
          />
        </SettingRow>
        <SettingRow label="Access key" htmlFor="s3-key">
          <TextSetting
            id="s3-key"
            value={draft.s3.accessKey}
            onChange={(accessKey) => setDraft({ ...draft, s3: { ...draft.s3, accessKey } })}
            mono
          />
        </SettingRow>
        <SettingRow label="Secret" htmlFor="s3-secret" help="Never sent back. Leave blank to keep the stored one.">
          <SecretSetting
            id="s3-secret"
            value={draft.s3.secret ?? ""}
            onChange={(secret) => setDraft({ ...draft, s3: { ...draft.s3, secret } })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow
          label="Path-style URLs"
          htmlFor="s3-path"
          help="Needed by MinIO and some self-hosted gateways that don't support virtual-hosted buckets."
        >
          <ToggleSetting
            id="s3-path"
            checked={draft.s3.forcePathStyle}
            onChange={(forcePathStyle) => setDraft({ ...draft, s3: { ...draft.s3, forcePathStyle } })}
            label={draft.s3.forcePathStyle ? "bucket in the path" : "bucket in the hostname"}
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSaveBar
        dirty={isDirty(draft, settings)}
        pending={save.isPending}
        errors={errors}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </div>
  );
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}
