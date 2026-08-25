import { useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
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
import { nodeRotateTokenActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
} from "./assistant-action-dialog-utils";
import {
  assistantNodeEffectResponseSchema,
  oneTimeMaterialUnavailable,
  readNodeAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantNodeRotateTokenParams {
  readonly nodeId: string;
}

export function AssistantNodeRotateTokenDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantNodeRotateTokenParams;
  readonly onComplete: (nodeId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{
    id: string;
    authToken?: string;
    signingSecret?: string;
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
      const expected = nodeRotateTokenActionParamsSchema.parse(params);
      const before = await readNodeAuthorization(expected.nodeId);
      if (before.id !== expected.nodeId || before.lifecycle !== "active") {
        throw new Error(
          "NyxID could not verify the active node before rotation.",
        );
      }
      const response = assistantNodeEffectResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/nodes/rotate-token", {
          actionRequestId,
          nodeId: expected.nodeId,
        }),
      );
      const after = await readNodeAuthorization(response.resource.nodeId);
      if (
        after.id !== expected.nodeId ||
        !after.is_active ||
        (!response.replayed &&
          (after.state_version !== before.state_version + 1 ||
            after.access_revision !== before.access_revision + 1 ||
            !isNewerTimestamp(before.updated_at, after.updated_at)))
      ) {
        throw new Error("NyxID could not verify the node credential rotation.");
      }
      setResult({
        id: after.id,
        ...(response.authToken ? { authToken: response.authToken } : {}),
        ...(response.signingSecret
          ? { signingSecret: response.signingSecret }
          : {}),
        unavailable: oneTimeMaterialUnavailable(
          response.oneTimeMaterial,
          Boolean(response.authToken && response.signingSecret),
        ),
      });
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not rotate this node token."));
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
            <RefreshCw className="size-4" />
            {result ? "Node credentials rotated" : "Rotate node credentials"}
          </DialogTitle>
          <DialogDescription>
            {result?.unavailable
              ? "The credentials were rotated, but the one-time replacements were not captured. Rotate them again before reconnecting the node."
              : result
                ? "These values are shown only once. Copy and store both securely now."
                : "Rotation immediately invalidates the node's current token and signing secret."}
          </DialogDescription>
        </DialogHeader>

        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {result.authToken ? (
              <div className="space-y-1.5">
                <Label htmlFor="assistant-node-auth-token">
                  One-time auth token
                </Label>
                <Input
                  id="assistant-node-auth-token"
                  readOnly
                  value={result.authToken}
                  className="font-mono text-xs"
                />
              </div>
            ) : null}
            {result.signingSecret ? (
              <div className="space-y-1.5">
                <Label htmlFor="assistant-node-signing-secret">
                  One-time signing secret
                </Label>
                <Input
                  id="assistant-node-signing-secret"
                  readOnly
                  value={result.signingSecret}
                  className="font-mono text-xs"
                />
              </div>
            ) : null}
          </div>
        ) : (
          <div className="flex items-center justify-between gap-4 border-y border-border py-4 text-[12px]">
            <span className="text-muted-foreground">Node</span>
            <Badge
              variant="secondary"
              className="max-w-[70%] truncate font-mono"
            >
              {params.nodeId}
            </Badge>
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
                disabled={submitting}
                onClick={() => void submit()}
              >
                Rotate credentials
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
