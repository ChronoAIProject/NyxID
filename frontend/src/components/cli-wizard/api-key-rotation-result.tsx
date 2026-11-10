import { useWizardTranslation } from "@/i18n/wizard";
import { Button } from "@/components/ui/button";
import { DisplayOncePanel } from "@/components/cli-wizard/display-once-panel";
import type { ApiKeyRotateSuccess } from "@/components/cli-wizard/confirm-panels";

export function ApiKeyRotationResult({
  result,
  description,
  ackButtonLabel,
  onAcknowledge,
}: {
  readonly result: ApiKeyRotateSuccess;
  readonly description: string;
  readonly ackButtonLabel: string;
  readonly onAcknowledge: () => void;
}) {
  const { t } = useWizardTranslation();
  if (result.full_key === "" && result.platform === "nyxid-assistant") {
    return (
      <div className="flex flex-col gap-6">
        <div className="flex flex-col gap-2">
          <h2 className="font-serif text-28 font-normal">{t("API key rotated")}</h2>
          <p className="text-12 text-muted-foreground">{t("Key rotated. This key is managed by the NyxID assistant; the new secret is stored encrypted on the server and is never shown.")}</p>
        </div>
        <Button variant="primary" onClick={onAcknowledge} className="w-full">{t("Close")}</Button>
      </div>
    );
  }
  return (
    <DisplayOncePanel
      title={t("API key rotated")}
      description={description}
      secret={result.full_key}
      ackButtonLabel={ackButtonLabel}
      onAcknowledge={onAcknowledge}
      isAcknowledging={false}
    />
  );
}
