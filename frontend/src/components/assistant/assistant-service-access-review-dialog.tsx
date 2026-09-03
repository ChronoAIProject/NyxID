import { useRef, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage } from "@/components/assistant/assistant-action-dialog-utils";
import { api } from "@/lib/api-client";
import {
  assistantServiceAccessReviewRequestSchema,
  assistantServiceAccessReviewResponseSchema,
} from "@/schemas/assistant-action-effects";
import { serviceAccessReviewActionParamsSchema } from "@/schemas/assistant-actions";

export interface AssistantServiceAccessReviewParams {
  readonly userServiceId: string;
  readonly serviceSlug: string;
  readonly resourceUri: string;
}

export function AssistantServiceAccessReviewDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantServiceAccessReviewParams;
  readonly onComplete: (userServiceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [resultServiceId, setResultServiceId] = useState<string | null>(null);
  const [replayed, setReplayed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    submittingRef.current = false;
    setSubmitting(false);
    setResultServiceId(null);
    setReplayed(false);
    setError(null);
    onOpenChange(false);
  }

  async function submit() {
    if (submittingRef.current || resultServiceId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = serviceAccessReviewActionParamsSchema.parse({
        serviceAccessReview: params,
      }).serviceAccessReview;
      const request = assistantServiceAccessReviewRequestSchema.parse({
        actionRequestId,
        userServiceId: expected.userServiceId,
        serviceSlug: expected.serviceSlug,
        resourceUri: expected.resourceUri,
      });
      const response = assistantServiceAccessReviewResponseSchema.parse(
        await api.post<unknown>(
          "/assistant/actions/services/access-review",
          request,
        ),
      );
      if (response.resource.userServiceId !== expected.userServiceId) {
        throw new Error("NyxID returned a different service identity.");
      }
      setReplayed(response.replayed);
      setResultServiceId(response.resource.userServiceId);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not approve this service access."),
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
            {resultServiceId
              ? replayed
                ? "Access was already approved"
                : "Access approved"
              : "Review service access"}
          </DialogTitle>
          <DialogDescription>
            {resultServiceId
              ? "NyxID verified the connection grant. The assistant receives only the service reference."
              : "Approve this connection for the current assistant consent. Your credential remains in NyxID."}
          </DialogDescription>
        </DialogHeader>

        <div className="flex items-start gap-3 border-y border-border py-4">
          <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-nyx-secondary-400" />
          <div className="min-w-0 space-y-1">
            <p className="text-[13px] font-medium">{params.serviceSlug}</p>
            <p className="break-all font-mono text-[11px] text-muted-foreground">
              {params.userServiceId}
            </p>
          </div>
        </div>

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        <DialogFooter>
          {resultServiceId ? (
            <Button
              type="button"
              variant="primary"
              onClick={() => {
                onComplete(resultServiceId);
                close();
              }}
            >
              Return to chat
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
                <ShieldCheck />
                Approve access
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
