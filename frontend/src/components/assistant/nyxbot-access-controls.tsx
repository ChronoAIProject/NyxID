import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { GroupedSelectTrigger } from "@/components/ui/grouped-list";
import { useState } from "react";
import { ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useChangeNyxAgentModel,
  useNyxAgentModels,
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import {
  ACCESS_MODE_DESCRIPTION,
  ACCESS_MODE_LABEL,
  effectiveAccessMode,
  modelDisplayName,
  modelCannotViewImages,
} from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import {
  nyxAgentAccessModeSchema,
  type NyxAgentAccessMode,
  type NyxAgentModel,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

const ACCESS_MODES = nyxAgentAccessModeSchema.options;

/** The access mode that governs an agent's actions, with what it means. */
export function AccessModeBadge({
  mode,
  overridden = false,
  className,
}: {
  readonly mode: NyxAgentAccessMode;
  /** The agent has its own mode instead of the workspace mode. */
  readonly overridden?: boolean;
  readonly className?: string;
}) {
  useUiTranslation();
  return (
    <Badge
      variant={mode === "autopilot" ? "warning" : "secondary"}
      className={cn("gap-1", className)}
      title={ACCESS_MODE_DESCRIPTION[mode]}
      aria-label={translateUi("Access mode: {{value1}}{{value2}}", { value1: ACCESS_MODE_LABEL[mode], value2: overridden ? " (this agent)" : "" })}
    >
      <ShieldCheck aria-hidden="true" className="h-3 w-3" />
      {ACCESS_MODE_LABEL[mode]}
      {overridden ? translateUi(" · agent") : ""}
    </Badge>
  );
}

/** The owner's workspace access mode, readable and switchable in place. */
export function AccessModeSwitcher({
  agentOverride,
  disabled,
}: {
  /** The thread's agent's own mode; when set it governs instead. */
  readonly agentOverride?: NyxAgentAccessMode | null;
  readonly disabled?: boolean;
}) {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const update = useUpdateNyxBotSettings();
  const [error, setError] = useState<string>();
  const workspace = settings.data?.access_mode;
  if (!workspace) return null;
  if (agentOverride) {
    return <AccessModeBadge mode={effectiveAccessMode(workspace, agentOverride)} overridden />;
  }
  return (
    <div className="flex items-center gap-1">
      <Select
        value={workspace}
        disabled={disabled || update.isPending}
        onValueChange={(value) => {
          setError(undefined);
          update
            .mutateAsync({ access_mode: nyxAgentAccessModeSchema.parse(value) })
            .catch((cause: unknown) => {
              setError(errorMessage(cause, "Could not change the access mode."));
            });
        }}
      >
        <SelectTrigger aria-label={translateUi("Access mode")} className="h-7 w-auto gap-1 px-2 text-14">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {ACCESS_MODES.map((mode) => (
            <SelectItem key={mode} value={mode}>
              {ACCESS_MODE_LABEL[mode]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {error ? (
        <span role="alert" className="text-13 text-destructive">
          {translateUi(error)}
        </span>
      ) : null}
    </div>
  );
}

/**
 * The live model list as select items. A current model NyxAgent no longer
 * offers stays visible, marked unavailable, so the selection is never hidden.
 */
export function modelOptions(
  models: readonly NyxAgentModel[] | undefined,
  current: string | undefined,
): NyxAgentModel[] {
  const rows = [...(models ?? [])];
  if (current && !rows.some((row) => row.id === current)) {
    rows.unshift({
      id: current,
      label: current.replace(/^nyxagent\//, ""),
      available: false,
    });
  }
  return rows;
}

export function ModelSelect({
  value,
  onChange,
  disabled,
  label = "Model",
  className,
  grouped = false,
  agentId,
  conversationId,
}: {
  readonly grouped?: boolean;
  readonly agentId?: string;
  readonly conversationId?: string;
  readonly value: string | undefined;
  readonly onChange: (model: string) => void;
  readonly disabled?: boolean;
  readonly label?: string;
  readonly className?: string;
}) {
  useUiTranslation();
  const Trigger = grouped ? GroupedSelectTrigger : SelectTrigger;
  const models = useNyxAgentModels(true, { agentId, conversationId });
  const options = modelOptions(models.data, value);
  return (
    <Select
      value={value ?? ""}
      disabled={disabled || models.isPending}
      onValueChange={onChange}
    >
      <Trigger aria-label={label} className={className}>
        <SelectValue
          placeholder={
            models.isPending ? translateUi("Loading models...") : value === "" ? translateUi("Model unavailable") : translateUi("Choose a model")
          }
        />
      </Trigger>
      <SelectContent>
        {options.map((row) => (
          <SelectItem key={row.id} value={row.id} disabled={!row.available}>
            {row.available
              ? modelDisplayName(row)
              : translateUi("{{value1}} (unavailable)", { value1: modelDisplayName(row) })}
            {row.id.startsWith("nyxid/") && row.description && (
              <span className="block text-13 text-text-tertiary">{row.description}</span>
            )}
            {modelCannotViewImages(row) && (
              <span className="ml-2 text-13 text-text-tertiary">{translateUi("Can’t view images")}</span>
            )}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** A thread's model selector; the server refuses unavailable models and busy threads. */
export function ThreadModelSelect({
  conversationId,
  model,
  disabled,
}: {
  readonly conversationId: string;
  readonly model: string | undefined;
  readonly disabled?: boolean;
}) {
  useUiTranslation();
  const change = useChangeNyxAgentModel();
  const [error, setError] = useState<string>();
  return (
    <div className="flex items-center gap-1">
      <ModelSelect
        value={model}
        disabled={disabled || change.isPending}
        className="h-7 w-auto gap-1 px-2 text-13"
        onChange={(next) => {
          if (next === model) return;
          setError(undefined);
          change.mutateAsync({ conversationId, model: next }).catch((cause: unknown) => {
            setError(errorMessage(cause, "Could not change the model."));
          });
        }}
      />
      {error ? (
        <span role="alert" className="text-13 text-destructive">
          {translateUi(error)}
        </span>
      ) : null}
    </div>
  );
}
