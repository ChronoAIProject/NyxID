import { useState } from "react";
import { Info, X } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useDirectConversationSettings,
  useDirectEfforts,
  useDirectModels,
  useDirectSkills,
} from "@/hooks/use-assistant-direct";

const BANNER_DISMISSED_KEY = "nyxid.direct-chat-banner-dismissed";
export const DIRECT_MODE_COPY =
  "Direct model chat — no tools, no approvals, and conversations are not saved.";

export function DirectModeBanner() {
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(BANNER_DISMISSED_KEY) === "true",
  );
  if (dismissed) return null;

  return (
    <div className="flex min-h-9 shrink-0 items-center gap-2 border-b border-border/60 bg-overlay px-4 py-2 text-[11px] text-muted-foreground sm:px-6">
      <Info aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 flex-1">{DIRECT_MODE_COPY}</span>
      <button
        type="button"
        aria-label="Dismiss direct chat notice"
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-text-tertiary hover:bg-overlay-strong hover:text-foreground focus-visible:outline-none"
        onClick={() => {
          localStorage.setItem(BANNER_DISMISSED_KEY, "true");
          setDismissed(true);
        }}
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

export function DirectChatControls({
  conversationId,
  disabled = false,
}: {
  readonly conversationId: string | undefined;
  readonly disabled?: boolean;
}) {
  const models = useDirectModels();
  const skills = useDirectSkills();
  const efforts = useDirectEfforts();
  const defaultModel =
    models.data?.find((model) => model.default)?.id ?? models.data?.[0]?.id;
  const { settings, canUpdate, setModel, setSkill, setEffort } =
    useDirectConversationSettings(conversationId, defaultModel);

  return (
    <div className="ml-[30px] flex min-w-0 flex-wrap items-start gap-2 pb-1.5">
      <div className="w-[150px] min-w-0">
        <label
          htmlFor="direct-chat-model"
          className="mb-1 block text-[10px] font-medium text-text-tertiary"
        >
          Model
        </label>
        <Select
          value={settings.model}
          onValueChange={setModel}
          disabled={disabled || !canUpdate || models.isLoading}
        >
          <SelectTrigger id="direct-chat-model" className="h-7 rounded-md px-2">
            <SelectValue placeholder="Choose model" />
          </SelectTrigger>
          <SelectContent>
            {(models.data ?? []).map((model) => (
              <SelectItem key={model.id} value={model.id}>
                {model.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="w-[150px] min-w-0">
        <label
          htmlFor="direct-chat-effort"
          className="mb-1 block text-[10px] font-medium text-text-tertiary"
        >
          Reasoning effort
        </label>
        <Select
          value={settings.effort ?? "default"}
          onValueChange={(value) =>
            setEffort(value === "default" ? null : value)
          }
          disabled={disabled || !canUpdate || efforts.isLoading}
        >
          <SelectTrigger
            id="direct-chat-effort"
            className="h-7 rounded-md px-2"
          >
            <SelectValue placeholder="Model default" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">Model default</SelectItem>
            {(efforts.data ?? []).map((effort) => (
              <SelectItem key={effort.id} value={effort.id}>
                {effort.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="min-w-[190px] flex-1">
        <label
          htmlFor="direct-chat-skill"
          className="mb-1 block text-[10px] font-medium text-text-tertiary"
        >
          NyxID knowledge
        </label>
        <Select
          value={settings.skillSlug ?? "none"}
          onValueChange={(value) => setSkill(value === "none" ? null : value)}
          disabled={disabled || !canUpdate || skills.isLoading}
        >
          <SelectTrigger id="direct-chat-skill" className="h-7 rounded-md px-2">
            <SelectValue placeholder="No skill" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="none">No skill</SelectItem>
            {(skills.data ?? []).map((skill) => (
              <SelectItem key={skill.slug} value={skill.slug}>
                {skill.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="mt-1 text-[10px] leading-4 text-text-tertiary">
          A skill teaches the model about NyxID; it cannot take actions here.
        </p>
      </div>
    </div>
  );
}
