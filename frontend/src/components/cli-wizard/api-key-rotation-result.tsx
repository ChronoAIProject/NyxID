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
  if (result.full_key === "" && result.platform === "nyxid-assistant") {
    return (
      <div className="flex flex-col gap-6">
        <div className="flex flex-col gap-2">
          <h2 className="font-serif text-[28px] font-normal">API key rotated</h2>
          <p className="text-[12px] text-muted-foreground">
            Key rotated. This key is managed by the NyxID assistant; the new secret is stored
            encrypted on the server and is never shown.
          </p>
        </div>
        <Button variant="primary" onClick={onAcknowledge} className="w-full">
          Close
        </Button>
      </div>
    );
  }
  return (
    <DisplayOncePanel
      title="API key rotated"
      description={description}
      secret={result.full_key}
      ackButtonLabel={ackButtonLabel}
      onAcknowledge={onAcknowledge}
      isAcknowledging={false}
    />
  );
}
