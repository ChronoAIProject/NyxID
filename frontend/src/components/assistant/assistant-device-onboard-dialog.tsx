import { useRef, useState } from "react";
import { QrCode } from "lucide-react";
import QRCode from "qrcode";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import { deviceOnboardActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
} from "./assistant-action-dialog-utils";
import {
  assistantDeviceEffectResponseSchema,
  oneTimeMaterialUnavailable,
  readDeviceAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantDeviceOnboardParams {
  readonly label: string;
  readonly targetOrgId?: string;
  readonly defaultServiceIds?: readonly string[];
}

export function AssistantDeviceOnboardDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantDeviceOnboardParams;
  readonly onComplete: (deviceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [label, setLabel] = useState(params.label);
  const [targetOrgId, setTargetOrgId] = useState(params.targetOrgId ?? "");
  const [defaultServiceIds, setDefaultServiceIds] = useState(
    params.defaultServiceIds?.join(", ") ?? "",
  );
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [result, setResult] = useState<{
    id: string;
    qrPayload?: string;
    expiresAt?: string;
    unavailable: boolean;
  } | null>(null);

  function close() {
    submittingRef.current = false;
    setSubmitting(false);
    setError(null);
    setQrDataUrl(null);
    setResult(null);
    onOpenChange(false);
  }

  async function submit() {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const reviewed = deviceOnboardActionParamsSchema.parse({
        label,
        ...(targetOrgId.trim() ? { targetOrgId: targetOrgId.trim() } : {}),
        ...(defaultServiceIds.trim()
          ? {
              defaultServiceIds: defaultServiceIds
                .split(",")
                .map((value) => value.trim())
                .filter(Boolean),
            }
          : {}),
      });
      const response = assistantDeviceEffectResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/nodes/device-onboard", {
          actionRequestId,
          label: reviewed.label,
          targetOrgId: reviewed.targetOrgId,
          defaultServiceIds: reviewed.defaultServiceIds,
        }),
      );
      const evidence = await readDeviceAuthorization(
        response.resource.deviceId,
      );
      if (
        evidence.id !== response.resource.deviceId ||
        evidence.used ||
        evidence.redeemed_node_id !== null ||
        (reviewed.targetOrgId &&
          evidence.owner_user_id !== reviewed.targetOrgId)
      ) {
        throw new Error(
          "NyxID could not verify the unused device onboarding package.",
        );
      }
      if (response.qrPayload) {
        try {
          setQrDataUrl(
            await QRCode.toDataURL(response.qrPayload, {
              errorCorrectionLevel: "M",
              margin: 2,
              width: 256,
            }),
          );
        } catch {
          setQrDataUrl(null);
        }
      }
      setResult({
        id: evidence.id,
        ...(response.qrPayload ? { qrPayload: response.qrPayload } : {}),
        ...(response.expiresAt ? { expiresAt: response.expiresAt } : {}),
        unavailable: oneTimeMaterialUnavailable(
          response.oneTimeMaterial,
          Boolean(response.qrPayload),
        ),
      });
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not onboard this device."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <QrCode className="size-4" />
            {result ? "Device onboarding ready" : "Onboard headless device"}
          </DialogTitle>
          <DialogDescription>
            {result?.unavailable
              ? "The onboarding package was created, but its one-time QR credential was not captured. Create another onboarding package before provisioning the device."
              : result
                ? "This provisioning credential is shown only once. Scan or store it securely now."
                : "Review the device owner and initial service allowlist before creating the provisioning package."}
          </DialogDescription>
        </DialogHeader>

        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {qrDataUrl ? (
              <img
                src={qrDataUrl}
                alt="One-time device onboarding QR code"
                className="mx-auto size-56 rounded-lg bg-white p-2"
              />
            ) : null}
            {result.qrPayload ? (
              <div className="space-y-1.5">
                <Label htmlFor="assistant-device-qr-payload">
                  One-time provisioning payload
                </Label>
                <Input
                  id="assistant-device-qr-payload"
                  readOnly
                  value={result.qrPayload}
                  className="font-mono text-xs"
                />
              </div>
            ) : null}
            {result.expiresAt ? (
              <p className="text-[11px] text-muted-foreground">
                Expires {result.expiresAt}
              </p>
            ) : null}
          </div>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            <div className="space-y-1.5">
              <Label htmlFor="assistant-device-label">Device label</Label>
              <Input
                id="assistant-device-label"
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-device-owner">
                Organization owner (optional)
              </Label>
              <Input
                id="assistant-device-owner"
                value={targetOrgId}
                onChange={(event) => setTargetOrgId(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-device-services">
                Default service IDs (comma-separated)
              </Label>
              <Input
                id="assistant-device-services"
                value={defaultServiceIds}
                onChange={(event) => setDefaultServiceIds(event.target.value)}
              />
            </div>
          </div>
        )}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          {result ? (
            <Button
              type="button"
              variant="primary"
              onClick={() => {
                onComplete(result.id);
                close();
              }}
            >
              {result.unavailable ? "Acknowledge" : "I have saved it"}
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="primary"
                isLoading={submitting}
                disabled={submitting || !label.trim()}
                onClick={() => void submit()}
              >
                Create onboarding package
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
