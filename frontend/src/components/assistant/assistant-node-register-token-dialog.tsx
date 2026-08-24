import { useRef, useState } from "react";
import { KeyRound } from "lucide-react";
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
import { nodeRegisterTokenActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
} from "./assistant-action-dialog-utils";
import {
  assistantNodeEffectResponseSchema,
  oneTimeMaterialUnavailable,
  readNodeAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantNodeRegisterTokenParams {
  readonly name: string;
  readonly targetOrgId?: string;
}

export function AssistantNodeRegisterTokenDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantNodeRegisterTokenParams;
  readonly onComplete: (nodeId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [name, setName] = useState(params.name);
  const [targetOrgId, setTargetOrgId] = useState(params.targetOrgId ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{
    id: string;
    token?: string;
    expiresAt?: string;
    unavailable: boolean;
  } | null>(null);

  function close() {
    submittingRef.current = false;
    setSubmitting(false);
    setError(null);
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
      const reviewed = nodeRegisterTokenActionParamsSchema.parse({
        name,
        ...(targetOrgId.trim() ? { targetOrgId: targetOrgId.trim() } : {}),
      });
      const response = assistantNodeEffectResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/nodes/register-token", {
          actionRequestId,
          name: reviewed.name,
          targetOrgId: reviewed.targetOrgId,
        }),
      );
      const evidence = await readNodeAuthorization(response.resource.nodeId);
      if (
        evidence.id !== response.resource.nodeId ||
        !evidence.is_active ||
        (evidence.lifecycle !== "registration_pending" &&
          evidence.lifecycle !== "active") ||
        (reviewed.targetOrgId &&
          evidence.owner_user_id !== reviewed.targetOrgId)
      ) {
        throw new Error(
          "NyxID could not verify the node registration authority.",
        );
      }
      setResult({
        id: evidence.id,
        ...(response.registrationToken
          ? { token: response.registrationToken }
          : {}),
        ...(response.expiresAt ? { expiresAt: response.expiresAt } : {}),
        unavailable: oneTimeMaterialUnavailable(
          response.oneTimeMaterial,
          Boolean(response.registrationToken),
        ),
      });
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not create this registration token."),
      );
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
            <KeyRound className="size-4" />
            {result
              ? "Node registration ready"
              : "Create node registration token"}
          </DialogTitle>
          <DialogDescription>
            {result?.unavailable
              ? "The registration was created, but its one-time token was not captured. Create another registration token before continuing setup."
              : result
                ? "This token is shown only once. Copy and store it securely now."
                : "Review the node name and owner. NyxID creates the token only after you continue."}
          </DialogDescription>
        </DialogHeader>

        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {result.token ? (
              <div className="space-y-1.5">
                <Label htmlFor="assistant-node-registration-token">
                  One-time registration token
                </Label>
                <Input
                  id="assistant-node-registration-token"
                  readOnly
                  value={result.token}
                  className="font-mono text-xs"
                />
              </div>
            ) : null}
            {result.expiresAt ? (
              <p className="text-[11px] text-muted-foreground">
                Expires {result.expiresAt}
              </p>
            ) : null}
            <p className="break-all font-mono text-[11px] text-muted-foreground">
              {result.id}
            </p>
          </div>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            <div className="space-y-1.5">
              <Label htmlFor="assistant-node-registration-name">
                Node name
              </Label>
              <Input
                id="assistant-node-registration-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoComplete="off"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-node-registration-owner">
                Organization owner (optional)
              </Label>
              <Input
                id="assistant-node-registration-owner"
                value={targetOrgId}
                onChange={(event) => setTargetOrgId(event.target.value)}
                autoComplete="off"
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
                disabled={submitting || !name.trim()}
                onClick={() => void submit()}
              >
                Create token
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
