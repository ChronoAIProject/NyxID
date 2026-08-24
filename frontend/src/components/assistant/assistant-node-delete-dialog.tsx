import { useRef, useState } from "react";
import { ShieldAlert } from "lucide-react";
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
import { nodeDeleteActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
  isNotFound,
} from "./assistant-action-dialog-utils";
import {
  assistantNodeEffectResponseSchema,
  readNodeAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantNodeDeleteParams {
  readonly nodeId: string;
}

export function AssistantNodeDeleteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantNodeDeleteParams;
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
      const expected = nodeDeleteActionParamsSchema.parse(params);
      const before = await readNodeAuthorization(expected.nodeId);
      if (before.id !== expected.nodeId || !before.is_active) {
        throw new Error(
          "NyxID could not verify the active node before deletion.",
        );
      }
      const response = assistantNodeEffectResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/nodes/delete", {
          actionRequestId,
          nodeId: expected.nodeId,
          expectedStateVersion: before.state_version,
        }),
      );
      try {
        await readNodeAuthorization(response.resource.nodeId);
        throw new Error(
          "NyxID still returned live node authorization evidence.",
        );
      } catch (caught) {
        if (!isNotFound(caught)) throw caught;
      }
      setResultId(response.resource.nodeId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not delete this node."));
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
            {resultId ? "Credential node deleted" : "Delete credential node"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "The node authorization projection is absent, confirming deletion."
              : "This deactivates the node, its bindings, and pending credential work. It cannot be undone from chat."}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Node</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.nodeId}
              </Badge>
            </div>
            <label className="flex items-start gap-2">
              <Checkbox
                checked={confirmed}
                onCheckedChange={(value) => setConfirmed(value === true)}
              />
              <span className="flex items-start gap-1.5 text-destructive">
                <ShieldAlert className="mt-0.5 size-3 shrink-0" />I understand
                this removes node routing and pending credential work.
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
                Delete node
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
