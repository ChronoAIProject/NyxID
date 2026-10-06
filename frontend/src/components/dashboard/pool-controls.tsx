import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { retryTriggers, type FailoverPolicy } from "@/schemas/pools";

export function Choice({
  label,
  value,
  onChange,
  options,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: readonly [string, string][];
  disabled?: boolean;
}) {
  return (
    <label className="space-y-1 text-[12px]">
      <span>{label}</span>
      <Select value={value} onValueChange={onChange} disabled={disabled}>
        <SelectTrigger aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([id, text]) => (
            <SelectItem key={id} value={id}>
              {text}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}
export function NumberInput({
  label,
  value,
  onChange,
  min = 0,
  max,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
}) {
  return (
    <label className="space-y-1 text-[12px]">
      <span>{label}</span>
      <Input
        aria-label={label}
        type="number"
        min={min}
        max={max}
        value={Number.isFinite(value) ? value : ""}
        onChange={(e) => onChange(e.target.valueAsNumber)}
      />
    </label>
  );
}
export function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3 text-[12px]">
      <span>{label}</span>
      <Switch aria-label={label} checked={checked} onCheckedChange={onChange} />
    </label>
  );
}
export function PolicyEditor({
  policy,
  onChange,
}: {
  policy: FailoverPolicy;
  onChange: (policy: FailoverPolicy) => void;
}) {
  const set = <K extends keyof FailoverPolicy>(
    key: K,
    value: FailoverPolicy[K],
  ) => onChange({ ...policy, [key]: value });
  return (
    <div className="space-y-4 rounded-xl border border-border/50 p-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <NumberInput
          label="Maximum attempts"
          min={1}
          max={5}
          value={policy.max_attempts}
          onChange={(v) => set("max_attempts", v)}
        />
        <NumberInput
          label="Replay body limit (bytes)"
          min={1}
          value={policy.max_replay_body_bytes}
          onChange={(v) => set("max_replay_body_bytes", v)}
        />
        <NumberInput
          label="Attempt timeout (ms)"
          min={1000}
          max={300000}
          value={policy.per_attempt_timeout_ms}
          onChange={(v) => set("per_attempt_timeout_ms", v)}
        />
        <NumberInput
          label="Overall deadline (ms)"
          min={1000}
          max={600000}
          value={policy.overall_deadline_ms}
          onChange={(v) => set("overall_deadline_ms", v)}
        />
        <NumberInput
          label="Base cooldown (ms)"
          min={1}
          value={policy.cooldown.base_ms}
          onChange={(v) => set("cooldown", { ...policy.cooldown, base_ms: v })}
        />
        <NumberInput
          label="Maximum cooldown (ms)"
          min={1}
          max={3600000}
          value={policy.cooldown.max_ms}
          onChange={(v) => set("cooldown", { ...policy.cooldown, max_ms: v })}
        />
        <NumberInput
          label="Failures before cooldown"
          min={1}
          value={policy.cooldown.failures_to_open}
          onChange={(v) =>
            set("cooldown", { ...policy.cooldown, failures_to_open: v })
          }
        />
      </div>
      <Toggle
        label="Honor provider Retry-After"
        checked={policy.cooldown.honor_retry_after}
        onChange={(v) =>
          set("cooldown", { ...policy.cooldown, honor_retry_after: v })
        }
      />
      <fieldset className="space-y-2">
        <legend className="text-[12px] font-medium">Retry causes</legend>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {retryTriggers.map((trigger) => (
            <label
              key={trigger}
              className="flex items-center gap-2 text-[12px]"
            >
              <Checkbox
                checked={policy.retry_on.includes(trigger)}
                onCheckedChange={(v) =>
                  set(
                    "retry_on",
                    v === true
                      ? [...policy.retry_on, trigger]
                      : policy.retry_on.filter((t) => t !== trigger),
                  )
                }
              />
              {trigger.replaceAll("_", " ")}
            </label>
          ))}
        </div>
      </fieldset>
      <Toggle
        label="Allow replay after an ambiguous dispatch"
        checked={policy.retry_ambiguous_dispatch}
        onChange={(v) => set("retry_ambiguous_dispatch", v)}
      />
      <p className="text-[12px] text-muted-foreground">
        A timeout or 5xx may follow completed work. Enabling replay for POST can
        duplicate provider work and charges. A provider 429 rejection can fall
        back without this option. Set maximum attempts to 1 to disable fallback.
      </p>
    </div>
  );
}
