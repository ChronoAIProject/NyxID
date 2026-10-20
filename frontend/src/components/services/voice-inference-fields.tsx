import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  VOICE_BILLING_METRICS,
  voiceMetadataSchema,
  type VoiceMetadata,
} from "@/schemas/platform-keys";
import { metricLabel } from "@/schemas/billing-metrics";

export function VoiceInferenceFields({
  value,
  onChange,
}: {
  value?: VoiceMetadata | null;
  onChange: (voice: VoiceMetadata | null) => void;
}) {
  const validation = value ? voiceMetadataSchema.safeParse(value) : null;
  return (
    <div className="space-y-3 rounded-lg border p-3">
      <label className="block text-xs">
        Voice protocol
        <select
          aria-label="Voice protocol"
          className="ml-2 rounded border bg-background p-1"
          value={value?.protocol ?? "none"}
          onChange={(e) => {
            const protocol = e.target.value as
              | VoiceMetadata["protocol"]
              | "none";
            onChange(
              protocol === "none"
                ? null
                : {
                    protocol,
                    models: value?.models ?? [],
                    voices: value?.voices ?? [],
                    usage_source:
                      protocol === "openai_live"
                        ? "provider_reported"
                        : "server_measured",
                    billing_metrics: value?.billing_metrics ?? [
                      "voice_seconds",
                    ],
                  },
            );
          }}
        >
          <option value="none">No voice metadata</option>
          <option value="openai_live">OpenAI Live</option>
          <option value="xai_realtime">xAI Realtime</option>
        </select>
      </label>
      {value && (
        <>
          {(["models", "voices"] as const).map((field) => (
            <fieldset key={field} className="space-y-2">
              <legend className="text-xs font-medium">
                {field === "models" ? "Voice models" : "Provider voices"}
              </legend>
              {value[field].map((item, index) => (
                <div key={index} className="flex flex-wrap items-center gap-2">
                  <Input
                    aria-label={`${field} ${index + 1} ID`}
                    value={item.id}
                    maxLength={128}
                    placeholder="Provider ID"
                    className="w-44"
                    onChange={(e) =>
                      onChange({
                        ...value,
                        [field]: value[field].map((row, i) =>
                          i === index ? { ...row, id: e.target.value } : row,
                        ),
                      })
                    }
                  />
                  <Input
                    aria-label={`${field} ${index + 1} label`}
                    value={item.label}
                    maxLength={128}
                    placeholder="Label"
                    className="w-44"
                    onChange={(e) =>
                      onChange({
                        ...value,
                        [field]: value[field].map((row, i) =>
                          i === index ? { ...row, label: e.target.value } : row,
                        ),
                      })
                    }
                  />
                  {field === "models" && (
                    <label className="text-xs">
                      <input
                        type="radio"
                        name="default-voice-model"
                        aria-label={`Default model ${index + 1}`}
                        checked={value.models[index]?.default ?? false}
                        onChange={() =>
                          onChange({
                            ...value,
                            models: value.models.map((m, i) => ({
                              ...m,
                              default: i === index,
                            })),
                          })
                        }
                      />{" "}
                      Default
                    </label>
                  )}
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={() =>
                      onChange({
                        ...value,
                        [field]: value[field].filter((_, i) => i !== index),
                      })
                    }
                  >
                    Remove {field === "models" ? "model" : "voice"}
                  </Button>
                </div>
              ))}
              <Button
                type="button"
                variant="outline"
                disabled={value[field].length >= (field === "models" ? 32 : 64)}
                onClick={() =>
                  onChange({
                    ...value,
                    [field]: [
                      ...value[field],
                      {
                        id: "",
                        label: "",
                        ...(field === "models"
                          ? { default: value.models.length === 0 }
                          : {}),
                      },
                    ],
                  })
                }
              >
                Add {field === "models" ? "voice model" : "provider voice"}
              </Button>
            </fieldset>
          ))}
          <label className="block text-xs">
            Usage source
            <select
              aria-label="Voice usage source"
              className="ml-2 rounded border bg-background p-1"
              value={value.usage_source}
              onChange={(e) =>
                onChange({
                  ...value,
                  usage_source: e.target.value as VoiceMetadata["usage_source"],
                })
              }
            >
              <option value="provider_reported">
                Provider reported (OpenAI)
              </option>
              <option value="server_measured">Server measured (xAI)</option>
            </select>
          </label>
          <fieldset className="flex flex-wrap gap-3 text-xs">
            <legend>Reported billing units</legend>
            {VOICE_BILLING_METRICS.map((metric) => (
              <label key={metric}>
                <input
                  type="checkbox"
                  checked={value.billing_metrics.includes(metric)}
                  disabled={metric === "voice_seconds"}
                  onChange={(e) =>
                    onChange({
                      ...value,
                      billing_metrics: e.target.checked
                        ? [...value.billing_metrics, metric]
                        : value.billing_metrics.filter((m) => m !== metric),
                    })
                  }
                />{" "}
                {metricLabel(metric)}
              </label>
            ))}
          </fieldset>
          <p className="text-xs text-muted-foreground">
            Models and voices must be available at the provider. Prices are
            configured separately below.
          </p>
          {validation && !validation.success && (
            <p role="alert" className="text-xs text-destructive">
              {validation.error.issues[0]?.message}
            </p>
          )}
        </>
      )}
    </div>
  );
}
