import { useRef, useState } from "react";
import { ArrowRight, ShieldAlert } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "@/lib/api-client";
import { nodeTransferActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
} from "./assistant-action-dialog-utils";
import {
  assistantNodeEffectResponseSchema,
  readNodeAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantNodeTransferParams {
  readonly nodeId: string;
  readonly newOwnerUserId: string;
}

export function AssistantNodeTransferDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantNodeTransferParams;
  readonly onComplete: (nodeId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [confirmed, setConfirmed] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);

  function close() {
    submittingRef.current = false;
    setConfirmed(false);
    setSubmitting(false);
    setError(null);
    setResultId(null);
    onOpenChange(false);
  }

  async function submit() {
    if (submittingRef.current || resultId || !confirmed) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const expected = nodeTransferActionParamsSchema.parse(params);
      const before = await readNodeAuthorization(expected.nodeId);
      if (before.id !== expected.nodeId || !before.is_active) {
        throw new Error(
          "NyxID could not verify the active node before transfer.",
        );
      }
      if (before.owner_user_id === expected.newOwnerUserId) {
        throw new Error("This node already belongs to the requested owner.");
      }
      const response = assistantNodeEffectResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/nodes/transfer", {
          actionRequestId,
          nodeId: expected.nodeId,
          newOwnerUserId: expected.newOwnerUserId,
          expectedStateVersion: before.state_version,
        }),
      );
      const after = await readNodeAuthorization(response.resource.nodeId);
      if (
        after.id !== expected.nodeId ||
        after.owner_user_id !== expected.newOwnerUserId ||
        !after.is_active ||
        (!response.replayed &&
          (after.state_version !== before.state_version + 1 ||
            !isNewerTimestamp(before.updated_at, after.updated_at)))
      ) {
        throw new Error("NyxID could not verify the node ownership transfer.");
      }
      setResultId(after.id);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not transfer this node."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {resultId
              ? "Credential node transferred"
              : "Transfer credential node"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "The canonical node projection confirms the new owner."
              : "The new owner receives control of this node, its routing authority, and its active bindings."}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-3">
              <Badge
                variant="secondary"
                className="max-w-[42%] truncate font-mono"
              >
                {params.nodeId}
              </Badge>
              <ArrowRight className="size-3 shrink-0 text-muted-foreground" />
              <Badge
                variant="secondary"
                className="max-w-[42%] truncate font-mono"
              >
                {params.newOwnerUserId}
              </Badge>
            </div>
            <label className="flex items-start gap-2">
              <Checkbox
                checked={confirmed}
                onCheckedChange={(value) => setConfirmed(value === true)}
              />
              <span className="flex items-start gap-1.5 text-destructive">
                <ShieldAlert className="mt-0.5 size-3 shrink-0" />I understand
                the current owner will lose control of this node.
              </span>
            </label>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          {resultId ? (
            <Button
              type="button"
              variant="primary"
              onClick={() => {
                onComplete(resultId);
                close();
              }}
            >
              Done
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="destructive"
                isLoading={submitting}
                disabled={submitting || !confirmed}
                onClick={() => void submit()}
              >
                Transfer node
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
