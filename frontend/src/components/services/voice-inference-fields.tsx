import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
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
  useUiTranslation();
  const validation = value ? voiceMetadataSchema.safeParse(value) : null;
  return (
    <div className="space-y-3 rounded-lg border p-3">
      <Label className="block">{translateUi("Voice protocol")}<select
          aria-label={translateUi("Voice protocol")}
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
                    delegation_model:
                      protocol === "openai_live"
                        ? value?.delegation_model
                        : null,
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
          <option value="none">{translateUi("No voice metadata")}</option>
          <option value="openai_live">{translateUi("OpenAI Live")}</option>
          <option value="xai_realtime">{translateUi("xAI Realtime")}</option>
        </select>
      </Label>
      {value && (
        <>
          {value.protocol === "openai_live" && (
            <Label className="block">{translateUi("Responses delegation model")}<Input
                aria-label={translateUi("Responses delegation model")}
                value={value.delegation_model ?? ""}
                maxLength={128}
                placeholder={translateUi("Provider model ID")}
                onChange={(e) =>
                  onChange({
                    ...value,
                    delegation_model: e.target.value || null,
                  })
                }
              />
              <span className="text-muted-foreground">{translateUi("Required for live conversation. Leave empty to offer dictation only.")}</span>
            </Label>
          )}
          {(["models", "voices"] as const).map((field) => (
            <fieldset key={field} className="space-y-2">
              <Label asChild><legend className="">
                {field === "models" ? translateUi("Voice models") : translateUi("Provider voices")}
              </legend></Label>
              {value[field].map((item, index) => (
                <div key={index} className="flex flex-wrap items-center gap-2">
                  <Input
                    aria-label={translateUi("{{value1}} {{value2}} ID", { value1: field, value2: index + 1 })}
                    value={item.id}
                    maxLength={128}
                    placeholder={translateUi("Provider ID")}
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
                    aria-label={translateUi("{{value1}} {{value2}} label", { value1: field, value2: index + 1 })}
                    value={item.label}
                    maxLength={128}
                    placeholder={translateUi("Label")}
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
                    <Label>
                      <input
                        type="radio"
                        name="default-voice-model"
                        aria-label={translateUi("Default model {{value1}}", { value1: index + 1 })}
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
                      />{" "}{translateUi("Default")}</Label>
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
                  >{translateUi("Remove ")}{field === "models" ? translateUi("model") : translateUi("voice")}
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
              >{translateUi("Add ")}{field === "models" ? translateUi("voice model") : translateUi("provider voice")}
              </Button>
            </fieldset>
          ))}
          <Label className="block">{translateUi("Usage source")}<select
              aria-label={translateUi("Voice usage source")}
              className="ml-2 rounded border bg-background p-1"
              value={value.usage_source}
              onChange={(e) =>
                onChange({
                  ...value,
                  usage_source: e.target.value as VoiceMetadata["usage_source"],
                })
              }
            >
              <option value="provider_reported">{translateUi("Provider reported (OpenAI)")}</option>
              <option value="server_measured">{translateUi("Server measured (xAI)")}</option>
            </select>
          </Label>
          <fieldset className="flex flex-wrap gap-3 text-13">
            <Label asChild><legend>{translateUi("Reported billing units")}</legend></Label>
            {VOICE_BILLING_METRICS.map((metric) => (
              <Label key={metric}>
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
              </Label>
            ))}
          </fieldset>
          <p className="text-13 text-muted-foreground">{translateUi("Models and voices must be available at the provider. Prices are configured separately below.")}</p>
          {validation && !validation.success && (
            <p role="alert" className="text-13 text-destructive">
              {validation.error.issues[0]?.message}
            </p>
          )}
        </>
      )}
    </div>
  );
}
