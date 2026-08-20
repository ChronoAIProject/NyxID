import { useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { z } from "zod";
import { GrantCascadeDialog } from "@/components/shared/grant-cascade-dialog";
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
  parseGrantCascadeDetails,
  type GrantCascadeDetails,
} from "@/schemas/oauth-revocation";
import {
  actionControlIdentitySchema,
  externalKeyDeleteActionParamsSchema,
} from "@/schemas/assistant-actions";

const externalKeyResourceSchema = z
  .object({ externalKeyId: actionControlIdentitySchema })
  .strict();
const assistantExternalKeyDeleteResponseSchema = z
  .object({
    resource: externalKeyResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

export interface AssistantExternalKeyDeleteParams {
  readonly externalKeyId: string;
}

export function AssistantExternalKeyDeleteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantExternalKeyDeleteParams;
  readonly onComplete: (externalKeyId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [cascadeDetails, setCascadeDetails] =
    useState<GrantCascadeDetails | null>(null);
  const [result, setResult] = useState<z.infer<
    typeof assistantExternalKeyDeleteResponseSchema
  > | null>(null);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setResult(null);
    setError(null);
    setVerified(false);
    setCascadeDetails(null);
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function verifyAbsence(externalKeyId: string): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      await api.get<unknown>(
        `/api-keys/external/${encodeURIComponent(externalKeyId)}/authorization`,
      );
      throw new Error(
        "NyxID still returned external-key evidence after delete.",
      );
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 404) {
        setVerified(true);
        setError(null);
      } else if (
        caught instanceof Error &&
        caught.message.startsWith("NyxID still")
      ) {
        setError(caught.message);
      } else {
        setError(
          errorMessage(
            caught,
            "NyxID could not verify the external credential was deleted.",
          ),
        );
      }
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function deleteKey(options?: {
    cascadeGrant?: boolean;
    grantScope?: "token";
  }) {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = externalKeyDeleteActionParamsSchema.parse(params);
      const response = assistantExternalKeyDeleteResponseSchema.parse(
        await api.post<unknown>(
          "/assistant/actions/endpoints/external-key-delete",
          {
            actionRequestId,
            externalKeyId: expected.externalKeyId,
            ...(options?.cascadeGrant ? { cascadeGrant: true } : {}),
            ...(options?.grantScope ? { grantScope: options.grantScope } : {}),
          },
        ),
      );
      setCascadeDetails(null);
      setResult(response);
      await verifyAbsence(response.resource.externalKeyId);
    } catch (caught) {
      const details =
        caught instanceof ApiError
          ? parseGrantCascadeDetails(caught.errorResponse)
          : null;
      if (details) {
        setCascadeDetails(details);
      } else {
        setError(
          errorMessage(
            caught,
            "NyxID could not delete this external credential.",
          ),
        );
      }
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  if (cascadeDetails) {
    return (
      <GrantCascadeDialog
        details={cascadeDetails}
        isPending={submitting}
        onCascade={() => void deleteKey({ cascadeGrant: true })}
        onRemoveOnly={() => void deleteKey({ grantScope: "token" })}
        onCancel={close}
      />
    );
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
            {result?.replayed
              ? "External credential already deleted"
              : "Delete external credential"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? "NyxID verified the credential is gone through its authorization evidence returning 404. This action is never remembered."
              : "This permanently deletes the stored credential. Confirm every time — NyxID will not remember this choice."}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">External key</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.externalKeyId}
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
            External credential absence verified.
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
                onClick={() => void deleteKey()}
              >
                Delete credential
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
                    void verifyAbsence(result.resource.externalKeyId)
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
                  onComplete(result.resource.externalKeyId);
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
