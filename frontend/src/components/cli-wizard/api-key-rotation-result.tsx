import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Button } from "@/components/ui/button";
import { DisplayOncePanel } from "@/components/cli-wizard/display-once-panel";
import type { ApiKeyRotateSuccess } from "@/components/cli-wizard/confirm-panels";
import { WizardHeading } from "./wizard-heading";

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
  useUiTranslation();
  if (result.full_key === "" && result.platform === "nyxid-assistant") {
    return (
      <div className="flex flex-col gap-6">
        <WizardHeading title={translateUi("API key rotated")}>{translateUi("Key rotated. This key is managed by the NyxID assistant; the new secret is stored encrypted on the server and is never shown.")}</WizardHeading>
        <Button variant="primary" onClick={onAcknowledge} className="w-full">{translateUi("Close")}</Button>
      </div>
    );
  }
  return (
    <DisplayOncePanel
      title={translateUi("API key rotated")}
      description={description}
      secret={result.full_key}
      ackButtonLabel={ackButtonLabel}
      onAcknowledge={onAcknowledge}
      isAcknowledging={false}
    />
  );
}
