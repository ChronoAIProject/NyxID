import { useState } from "react";
import { ShieldCheck, Cpu } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
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
  return (
    <Badge
      variant={mode === "autopilot" ? "warning" : "secondary"}
      className={cn("gap-1", className)}
      title={ACCESS_MODE_DESCRIPTION[mode]}
      aria-label={`Access mode: ${ACCESS_MODE_LABEL[mode]}${overridden ? " (this agent)" : ""}`}
    >
      <ShieldCheck aria-hidden="true" className="h-3 w-3" />
      {ACCESS_MODE_LABEL[mode]}
      {overridden ? " · this agent" : ""}
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
        <SelectTrigger aria-label="Permission mode" title={ACCESS_MODE_DESCRIPTION[workspace]} className={cn("h-7 w-auto min-w-0 gap-1 px-2 text-11", workspace === "autopilot" && "text-warning")}>
          <ShieldCheck aria-hidden="true" className="h-3 w-3 shrink-0" />
          <span className="truncate">{ACCESS_MODE_LABEL[workspace]}</span>
        </SelectTrigger>
        <SelectContent>
          {ACCESS_MODES.map((mode) => (
            <SelectItem key={mode} value={mode}>
              <span className="flex max-w-72 flex-col gap-0.5 whitespace-normal">
                <span className="text-12">{ACCESS_MODE_LABEL[mode]}</span>
                <span className="text-11 text-muted-foreground">{ACCESS_MODE_DESCRIPTION[mode]}</span>
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {error ? (
        <span role="alert" className="text-11 text-destructive">
          {error}
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
}: {
  readonly value: string | undefined;
  readonly onChange: (model: string) => void;
  readonly disabled?: boolean;
  readonly label?: string;
  readonly className?: string;
}) {
  const models = useNyxAgentModels();
  const options = modelOptions(models.data, value);
  return (
    <Select value={value ?? ""} disabled={disabled || models.isPending} onValueChange={onChange}>
      <SelectTrigger aria-label={label} className={className}>
        <Cpu aria-hidden="true" className="h-3 w-3 shrink-0" />
        <span className="shrink-0 text-text-tertiary">Model:</span>
        <span className="truncate">{options.find((row) => row.id === value)?.label ?? (models.isPending ? "Loading models..." : "Choose a model")}</span>
      </SelectTrigger>
      <SelectContent>
        {options.map((row) => (
          <SelectItem key={row.id} value={row.id} disabled={!row.available}>
            <span className="flex max-w-72 flex-col gap-0.5 whitespace-normal">
              <span className="text-12">{row.available ? row.label : `${row.label} (unavailable)`}</span>
              {row.description ? <span className="text-11 text-muted-foreground">{row.description}</span> : null}
            </span>
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
  const change = useChangeNyxAgentModel();
  const [error, setError] = useState<string>();
  return (
    <div className="flex items-center gap-1">
      <ModelSelect
        value={model}
        disabled={disabled || change.isPending}
        className="h-7 w-auto min-w-0 max-w-full gap-1 px-2 text-11"
        onChange={(next) => {
          if (next === model) return;
          setError(undefined);
          change.mutateAsync({ conversationId, model: next }).catch((cause: unknown) => {
            setError(errorMessage(cause, "Could not change the model."));
          });
        }}
      />
      {error ? (
        <span role="alert" className="text-11 text-destructive">
          {error}
        </span>
      ) : null}
    </div>
  );
}
