import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
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
import { pendingCredentialCancelActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
} from "./assistant-action-dialog-utils";
import {
  assistantPendingCredentialEffectResponseSchema,
  readPendingCredentialAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantPendingCredentialCancelParams {
  readonly nodeId: string;
  readonly pendingCredentialId: string;
}

export function AssistantPendingCredentialCancelDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantPendingCredentialCancelParams;
  readonly onComplete: (pendingCredentialId: string) => void;
}) {
  useUiTranslation();
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
      const expected = pendingCredentialCancelActionParamsSchema.parse(params);
      const before = await readPendingCredentialAuthorization(
        expected.nodeId,
        expected.pendingCredentialId,
      );
      if (!before.is_active || before.consumed_at || before.declined_at) {
        throw new Error("This pending credential request is no longer active.");
      }
      const response = assistantPendingCredentialEffectResponseSchema.parse(
        await api.post<unknown>(
          "/assistant/actions/nodes/pending-credential-cancel",
          {
            actionRequestId,
            nodeId: expected.nodeId,
            pendingCredentialId: expected.pendingCredentialId,
          },
        ),
      );
      const after = await readPendingCredentialAuthorization(
        expected.nodeId,
        response.resource.pendingCredentialId,
      );
      if (
        after.id !== expected.pendingCredentialId ||
        after.node_id !== expected.nodeId ||
        after.is_active ||
        !after.declined_at
      ) {
        throw new Error(
          "NyxID could not verify pending credential cancellation.",
        );
      }
      setResultId(after.id);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not cancel this pending credential."),
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
          <DialogTitle>
            {resultId
              ? translateUi("Pending credential cancelled")
              : translateUi("Cancel pending credential")}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? translateUi("The canonical projection confirms this request is inactive and declined.")
              : translateUi("The node will no longer be able to consume this pending credential request.")}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4 text-14">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">{translateUi("Pending credential")}</span>
              <Badge
                variant="secondary"
                className="max-w-[65%] truncate font-mono"
              >
                {params.pendingCredentialId}
              </Badge>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">{translateUi("Node")}</span>
              <Badge
                variant="secondary"
                className="max-w-[65%] truncate font-mono"
              >
                {params.nodeId}
              </Badge>
            </div>
            <Label className="flex items-start gap-2">
              <Checkbox
                checked={confirmed}
                onCheckedChange={(value) => setConfirmed(value === true)}
              />
              <span className="flex items-start gap-1.5 text-destructive">
                <ShieldAlert className="mt-0.5 size-3 shrink-0" />{translateUi("I understand this request cannot be consumed after cancellation.")}</span>
            </Label>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-13 text-destructive">
            {translateUi(error)}
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
            >{translateUi("Done")}</Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>{translateUi("Keep request")}</Button>
              <Button
                type="button"
                variant="destructive"
                isLoading={submitting}
                disabled={submitting || !confirmed}
                onClick={() => void submit()}
              >{translateUi("Cancel request")}</Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
