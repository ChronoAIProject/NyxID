import { useState, useSyncExternalStore, type ReactNode } from "react";
import { Check, ChevronDown, ChevronRight } from "lucide-react";
import { toast } from "sonner";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuPortal,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  useChangeNyxAgentModel,
  useNyxAgentModels,
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import {
  ACCESS_MODE_LABEL,
  modelDisplayName,
} from "@/lib/assistant/nyxbot-labels";
import type { NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";
import { modelOptions } from "./nyxbot-access-controls";

const MODES: readonly NyxAgentAccessMode[] = [
  "autopilot",
  "nyxmode",
  "ask_every_time",
];

/** One footer menu for the thread's model and effective action permissions. */
export function ComposerSettings({
  conversationId,
  model,
  onDraftModelChange,
  agentOverride,
  active = false,
  disabled = false,
  onVoiceSettings,
}: {
  readonly conversationId?: string;
  readonly model: string;
  readonly onDraftModelChange: (model: string) => void;
  readonly agentOverride?: NyxAgentAccessMode | null;
  readonly active?: boolean;
  readonly disabled?: boolean;
  readonly onVoiceSettings?: () => void;
}) {
  const advancedControls = useFeature(FEATURE_FLAG.ASSISTANT_MODEL_CONTROLS);
  const thinking = model === "nyxagent/research";
  const nextThinkingModel = thinking ? "nyxagent/chat" : "nyxagent/research";
  const menuLabel = advancedControls
    ? "Model and permissions"
    : "Thinking and permissions";
  const models = useNyxAgentModels();
  const settings = useNyxBotSettings();
  const changeModel = useChangeNyxAgentModel();
  const changePermissions = useUpdateNyxBotSettings();
  const options = modelOptions(models.data, model);
  const selected = options.find((row) => row.id === model);
  const name = modelDisplayName(selected ?? { id: model, label: model });
  const mode = agentOverride ?? settings.data?.access_mode;
  const accessLabel = mode ? ACCESS_MODE_LABEL[mode] : "Permissions";
  const triggerLabel = advancedControls
    ? name
    : `${accessLabel}${thinking ? ", Thinking" : ""}`;
  const modelLocked =
    active || changeModel.isPending || models.isPending || models.isError;
  const permissionsLocked =
    active || Boolean(agentOverride) || changePermissions.isPending || !settings.data;

  function selectModel(next: string) {
    if (next === model) return;
    if (!conversationId) {
      onDraftModelChange(next);
      return;
    }
    void changeModel
      .mutateAsync({ conversationId, model: next })
      .catch((error: unknown) => {
        toast.error(
          error instanceof Error
            ? error.message
            : "Could not change the model.",
        );
      });
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          aria-label={`${menuLabel}: ${triggerLabel}${advancedControls && mode ? `, ${accessLabel}` : ""}`}
          className="flex h-8 min-w-0 max-w-full items-center gap-1.5 rounded-lg px-2 text-12 text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
        >
          <span className="flex min-w-0 items-center text-foreground">
            <span className="truncate">
              {advancedControls ? name : accessLabel}
            </span>
            {!advancedControls && thinking && (
              <span className="shrink-0">, Thinking</span>
            )}
          </span>
          <ChevronDown aria-hidden className="h-3 w-3 shrink-0" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        side="top"
        align="end"
        sideOffset={6}
        collisionPadding={12}
        aria-label={menuLabel}
        className="w-56 max-w-[calc(100vw-2rem)] max-h-[var(--radix-dropdown-menu-content-available-height)] overflow-y-auto p-1"
      >
        {!advancedControls && (
          <DropdownMenuItem
            role="menuitemcheckbox"
            aria-label="Thinking mode"
            aria-checked={thinking}
            disabled={
              modelLocked ||
              !options.some(
                (row) => row.id === nextThinkingModel && row.available,
              )
            }
            className="min-h-8 px-2"
            onSelect={(event) => {
              event.preventDefault();
              selectModel(nextThinkingModel);
            }}
          >
            <span className="flex-1">Thinking</span>
            <span
              aria-hidden
              className={`flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 transition-colors ${thinking ? "bg-primary" : "bg-muted-foreground/30"}`}
            >
              <span
                className={`h-3 w-3 rounded-full bg-white transition-transform ${thinking ? "translate-x-3" : "translate-x-0"}`}
              />
            </span>
          </DropdownMenuItem>
        )}
        {!advancedControls && models.isError && (
          <DropdownMenuItem onSelect={() => void models.refetch()}>
            Retry loading settings
          </DropdownMenuItem>
        )}
        {!advancedControls && <DropdownMenuSeparator className="my-0.5" />}
        {advancedControls && (
          <>
            <DropdownMenuItem
              disabled
              className="min-h-8 px-2"
              title="NyxAgent does not expose configurable reasoning effort yet."
            >
              <span className="flex-1">Reasoning</span>
              <span className="text-muted-foreground">Unavailable</span>
              <ChevronRight aria-hidden />
            </DropdownMenuItem>
            <DropdownMenuSeparator className="my-0.5" />
            <SettingsSection label="Model" value={name}>
              {options.map((row) => (
                <DropdownMenuItem
                  key={row.id}
                  role="menuitemradio"
                  aria-checked={model === row.id}
                  disabled={modelLocked || !row.available}
                  onSelect={() => selectModel(row.id)}
                  className="min-h-8 px-2"
                >
                  <span className="min-w-0 flex-1 break-words">
                    {modelDisplayName(row)}
                    {!row.available && !models.isPending
                      ? " (unavailable)"
                      : ""}
                  </span>
                  {model === row.id && (
                    <Check aria-hidden className="shrink-0" />
                  )}
                </DropdownMenuItem>
              ))}
              {models.isError && (
                <DropdownMenuItem onSelect={() => void models.refetch()}>
                  Retry loading models
                </DropdownMenuItem>
              )}
            </SettingsSection>
            <DropdownMenuSeparator className="my-0.5" />
          </>
        )}
        <SettingsSection
          label="Permissions"
          value={mode ? ACCESS_MODE_LABEL[mode] : "Loading…"}
        >
          {MODES.map((value) => (
            <DropdownMenuItem
              key={value}
              role="menuitemradio"
              aria-checked={mode === value}
              disabled={permissionsLocked}
              className="min-h-8 px-2"
              onSelect={() => {
                if (value === mode) return;
                void changePermissions
                  .mutateAsync({ access_mode: value })
                  .catch((error: unknown) => {
                    toast.error(
                      error instanceof Error
                        ? error.message
                        : "Could not change permissions.",
                    );
                  });
              }}
            >
              <span className="flex-1">{ACCESS_MODE_LABEL[value]}</span>
              {mode === value && <Check aria-hidden />}
            </DropdownMenuItem>
          ))}
          {settings.isError && (
            <DropdownMenuItem onSelect={() => void settings.refetch()}>
              Retry loading permissions
            </DropdownMenuItem>
          )}
        </SettingsSection>
        {onVoiceSettings && <>
          <DropdownMenuSeparator className="my-0.5" />
          <DropdownMenuItem onSelect={onVoiceSettings}>Voice settings</DropdownMenuItem>
        </>}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const NARROW_MENU_QUERY = "(max-width: 639px)";

function subscribeToMenuWidth(onChange: () => void) {
  const query = window.matchMedia(NARROW_MENU_QUERY);
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}

/** Expand within the menu on phones, where adjacent flyouts cannot fit. */
function SettingsSection({
  label,
  value,
  children,
}: {
  readonly label: string;
  readonly value: string;
  readonly children: ReactNode;
}) {
  const narrow = useSyncExternalStore(
    subscribeToMenuWidth,
    () => window.matchMedia(NARROW_MENU_QUERY).matches,
    () => false,
  );
  const [expanded, setExpanded] = useState(false);
  const summary = (
    <>
      <span className="shrink-0">{label}</span>
      <span className="min-w-0 flex-1 truncate text-right text-muted-foreground">
        {value}
      </span>
      <ChevronRight
        aria-hidden
        className={`h-3.5 w-3.5 shrink-0 text-muted-foreground ${narrow && expanded ? "rotate-90" : ""}`}
      />
    </>
  );
  const triggerClass =
    "min-h-8 gap-2 rounded-md px-2 text-12 focus:bg-overlay-strong data-[state=open]:bg-overlay-strong";

  if (narrow) {
    return (
      <>
        <DropdownMenuItem
          className={triggerClass}
          aria-expanded={expanded}
          onSelect={(event) => {
            event.preventDefault();
            setExpanded(!expanded);
          }}
        >
          {summary}
        </DropdownMenuItem>
        {expanded && (
          <div
            role="group"
            aria-label={label}
            className="ml-2 border-l border-border pl-1"
          >
            {children}
          </div>
        )}
      </>
    );
  }

  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger className={triggerClass}>
        {summary}
      </DropdownMenuSubTrigger>
      <DropdownMenuPortal>
        <DropdownMenuSubContent
          sideOffset={6}
          collisionPadding={12}
          className="w-56 max-w-[calc(100vw-1.5rem)] max-h-[var(--radix-dropdown-menu-content-available-height)] overflow-y-auto bg-popover p-1"
        >
          {children}
        </DropdownMenuSubContent>
      </DropdownMenuPortal>
    </DropdownMenuSub>
  );
}
