import { useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { z } from "zod";
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
import { ApiError, api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  endpointDeleteActionParamsSchema,
} from "@/schemas/assistant-actions";

const endpointResourceSchema = z
  .object({ endpointId: actionControlIdentitySchema })
  .strict();
const assistantEndpointDeleteResponseSchema = z
  .object({
    resource: endpointResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

export interface AssistantEndpointDeleteParams {
  readonly endpointId: string;
}

export function AssistantEndpointDeleteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantEndpointDeleteParams;
  readonly onComplete: (endpointId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [result, setResult] = useState<z.infer<
    typeof assistantEndpointDeleteResponseSchema
  > | null>(null);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setResult(null);
    setError(null);
    setVerified(false);
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function verifyAbsence(endpointId: string): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      await api.get<unknown>(
        `/endpoints/${encodeURIComponent(endpointId)}/authorization`,
      );
      throw new Error("NyxID still returned endpoint evidence after delete.");
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 404) {
        setVerified(true);
        setError(null);
      } else if (caught instanceof Error && caught.message.startsWith("NyxID still")) {
        setError(caught.message);
      } else {
        setError(
          errorMessage(caught, "NyxID could not verify the endpoint was deleted."),
        );
      }
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function deleteEndpoint() {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = endpointDeleteActionParamsSchema.parse(params);
      const response = assistantEndpointDeleteResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/endpoints/delete", {
          actionRequestId,
          endpointId: expected.endpointId,
        }),
      );
      setResult(response);
      await verifyAbsence(response.resource.endpointId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not delete this endpoint."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {result?.replayed ? "Endpoint already deleted" : "Delete endpoint"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? "NyxID verified the endpoint is gone through its authorization evidence returning 404. This action is never remembered."
              : "This permanently deletes the endpoint. Confirm every time — NyxID will not remember this choice."}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Endpoint</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.endpointId}
              </Badge>
            </div>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        {verified ? (
          <p className="text-[11px] text-success">
            Endpoint absence verified.
          </p>
        ) : null}

        <DialogFooter>
          {!result ? (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="destructive"
                isLoading={submitting}
                disabled={submitting}
                onClick={() => void deleteEndpoint()}
              >
                Delete endpoint
              </Button>
            </>
          ) : (
            <>
              {!verified ? (
                <Button
                  type="button"
                  variant="outline"
                  isLoading={verifying}
                  onClick={() =>
                    void verifyAbsence(result.resource.endpointId)
                  }
                >
                  <RefreshCw />
                  Retry verification
                </Button>
              ) : null}
              <Button
                type="button"
                variant="primary"
                disabled={!verified}
                onClick={() => {
                  onComplete(result.resource.endpointId);
                  close();
                }}
              >
                Report deletion
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
