import { translateUi } from "@/i18n/use-ui-translation";
import { useUiTranslation } from "@/i18n/use-ui-translation";
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
  const { t } = useUiTranslation();
  const validation = value ? voiceMetadataSchema.safeParse(value) : null;
  return (
    <div className="space-y-3 rounded-lg border p-3">
      <label className="block text-xs">{t("Voice protocol")}<select
          aria-label={t("Voice protocol")}
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
          <option value="none">{t("No voice metadata")}</option>
          <option value="openai_live">{t("OpenAI Live")}</option>
          <option value="xai_realtime">{t("xAI Realtime")}</option>
        </select>
      </label>
      {value && (
        <>
          {(["models", "voices"] as const).map((field) => (
            <fieldset key={field} className="space-y-2">
              <legend className="text-xs font-medium">
                {field === "models" ? t("Voice models") : t("Provider voices")}
              </legend>
              {value[field].map((item, index) => (
                <div key={index} className="flex flex-wrap items-center gap-2">
                  <Input
                    aria-label={t("{{field}} {{value}} ID", { field: field, value: index + 1 })}
                    value={item.id}
                    maxLength={128}
                    placeholder={t("Provider ID")}
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
                    aria-label={t("{{field}} {{value}} label", { field: field, value: index + 1 })}
                    value={item.label}
                    maxLength={128}
                    placeholder={t("Label")}
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
                        aria-label={t("Default model {{value}}", { value: index + 1 })}
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
                      />{" "}{t("Default")}</label>
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
                  >{t("Remove ")}{field === "models" ? t("model") : t("voice")}
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
              >{t("Add ")}{field === "models" ? t("voice model") : t("provider voice")}
              </Button>
            </fieldset>
          ))}
          <label className="block text-xs">{t("Usage source")}<select
              aria-label={t("Voice usage source")}
              className="ml-2 rounded border bg-background p-1"
              value={value.usage_source}
              onChange={(e) =>
                onChange({
                  ...value,
                  usage_source: e.target.value as VoiceMetadata["usage_source"],
                })
              }
            >
              <option value="provider_reported">{t("Provider reported (OpenAI)")}</option>
              <option value="server_measured">{t("Server measured (xAI)")}</option>
            </select>
          </label>
          <fieldset className="flex flex-wrap gap-3 text-xs">
            <legend>{t("Reported billing units")}</legend>
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
          <p className="text-xs text-muted-foreground">{t("Models and voices must be available at the provider. Prices are configured separately below.")}</p>
          {validation && !validation.success && (
            <p role="alert" className="text-xs text-destructive">
              {translateUi(validation.error.issues[0]?.message ?? "")}
            </p>
          )}
        </>
      )}
    </div>
  );
}
