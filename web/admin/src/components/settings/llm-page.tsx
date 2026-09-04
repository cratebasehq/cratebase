import { useState } from "react";
import { toast } from "sonner";
import { useSettings, useSettingsMutation, type ServerSettings } from "@/hooks/use-settings";
import { Skeleton } from "@/components/ui/skeleton";
import {
  SecretSetting,
  SettingRow,
  SettingsSaveBar,
  SettingsSection,
  TextSetting,
  ToggleSetting,
} from "@/components/settings/settings-form";

type Draft = ServerSettings["llm"];

function draftOf(settings: ServerSettings): Draft {
  return { ...settings.llm };
}

/** Only send the api key when one was typed — an empty box means "keep
 * what is stored", same convention as `smtp.password`/`s3.secret`. */
function payloadOf(draft: Draft) {
  const llm: Record<string, unknown> = { ...draft };
  if (!draft.apiKey) delete llm.apiKey;
  return { llm };
}

function isDirty(draft: Draft, settings: ServerSettings): boolean {
  return JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
}

/**
 * `Settings.llm` (`crates/core/src/settings.rs`), consumed by
 * `POST /api/llm/chat` (`crates/server/src/llm.rs`) — the same provider
 * config a vector field's auto-embedding falls back to when its own
 * `embedding.provider` isn't `"echo"`. Disabled means requests are
 * answered by a deterministic, network-free echo provider instead of a
 * real model.
 */
export function LlmPage() {
  const { data: settings, isPending } = useSettings();
  const save = useSettingsMutation();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seedKey, setSeedKey] = useState<ServerSettings | undefined>(undefined);

  if (settings && (draft === null || (seedKey !== settings && !isDirty(draft, settings)))) {
    setSeedKey(settings);
    setDraft(draftOf(settings));
  }

  if (isPending || !draft || !settings) {
    return (
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  function submit() {
    if (!draft) return;
    save.mutate(payloadOf(draft), {
      onSuccess: () => {
        toast.success("Settings saved");
        // The secret was consumed; clear the box so it reads as "stored".
        setDraft((d) => (d ? { ...d, apiKey: "" } : d));
      },
      onError: (error) => {
        toast.error("Couldn't save LLM settings", { description: String(error) });
      },
    });
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-page">
      <SettingsSection
        title="LLM provider"
        description="Backs POST /api/llm/chat and any vector field whose auto-embedding provider isn't the echo test provider. Off answers with a deterministic, network-free echo provider instead of a real model."
      >
        <SettingRow label="Enabled" htmlFor="llm-enabled">
          <ToggleSetting
            id="llm-enabled"
            checked={draft.enabled}
            onChange={(enabled) => setDraft({ ...draft, enabled })}
            label={draft.enabled ? "Using the configured provider" : "Using the echo provider"}
          />
        </SettingRow>
        <SettingRow label="Provider" htmlFor="llm-provider" help='"openai" (or any OpenAI-compatible /chat/completions API, e.g. a local Ollama instance) is the only real provider today.'>
          <TextSetting
            id="llm-provider"
            value={draft.provider}
            onChange={(provider) => setDraft({ ...draft, provider })}
            placeholder="openai"
            mono
          />
        </SettingRow>
        <SettingRow label="Base URL" htmlFor="llm-base-url">
          <TextSetting
            id="llm-base-url"
            value={draft.baseUrl}
            onChange={(baseUrl) => setDraft({ ...draft, baseUrl })}
            placeholder="https://api.openai.com/v1"
            mono
          />
        </SettingRow>
        <SettingRow label="API key" htmlFor="llm-api-key" help="Never sent back by the server. Leave blank to keep the stored one.">
          <SecretSetting
            id="llm-api-key"
            value={draft.apiKey ?? ""}
            onChange={(apiKey) => setDraft({ ...draft, apiKey })}
            storedHint="•••••••• (unchanged)"
          />
        </SettingRow>
        <SettingRow label="Model" htmlFor="llm-model">
          <TextSetting
            id="llm-model"
            value={draft.model}
            onChange={(model) => setDraft({ ...draft, model })}
            placeholder="gpt-4o-mini"
            mono
          />
        </SettingRow>
      </SettingsSection>

      <SettingsSaveBar
        dirty={isDirty(draft, settings)}
        pending={save.isPending}
        errors={[]}
        onSave={submit}
        onReset={() => setDraft(draftOf(settings))}
      />
    </div>
  );
}
