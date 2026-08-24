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
import {
  assertSecretFreeReadBack,
  errorMessage,
} from "@/components/assistant/assistant-action-dialog-utils";
import { api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  connectionRevokeActionParamsSchema,
} from "@/schemas/assistant-actions";
import { assistantOneTimeMaterialSchema } from "@/schemas/assistant-action-effects";

const connectionEvidenceSchema = z
  .object({
    service_id: actionControlIdentitySchema,
    is_active: z.boolean(),
    state_version: z.number().int().nonnegative(),
    updated_at: z.string(),
  })
  .strict();
const connectionEffectSchema = z
  .object({
    resource: z
      .object({ serviceId: actionControlIdentitySchema })
      .strict(),
    replayed: z.boolean(),
    oneTimeMaterial: assistantOneTimeMaterialSchema,
  })
  .strict();

export type AssistantConnectionRevokeParams = z.infer<
  typeof connectionRevokeActionParamsSchema
>;

export function AssistantConnectionRevokeDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantConnectionRevokeParams;
  readonly onComplete: (serviceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verifyingRef = useRef(false);
  const expectedVersionRef = useRef<number | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [resultId, setResultId] = useState<string | null>(null);
  const [replayed, setReplayed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    submittingRef.current = false;
    verifyingRef.current = false;
    expectedVersionRef.current = null;
    setSubmitting(false);
    setVerifying(false);
    setVerified(false);
    setResultId(null);
    setReplayed(false);
    setError(null);
    onOpenChange(false);
  }

  async function readEvidence(serviceId: string) {
    const value = await api.get<unknown>(
      `/connections/${encodeURIComponent(serviceId)}/authorization`,
    );
    assertSecretFreeReadBack(value);
    const evidence = connectionEvidenceSchema.parse(value);
    if (evidence.service_id !== serviceId) {
      throw new Error("NyxID returned a different connection identity.");
    }
    return evidence;
  }

  async function verify(serviceId: string, expectedVersion: number) {
    if (verifyingRef.current) return;
    verifyingRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const evidence = await readEvidence(serviceId);
      if (evidence.is_active || evidence.state_version !== expectedVersion + 1) {
        throw new Error("NyxID connection evidence did not prove the revocation.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not verify this revocation."));
    } finally {
      verifyingRef.current = false;
      setVerifying(false);
    }
  }

  async function submit() {
    if (submittingRef.current || resultId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = connectionRevokeActionParamsSchema.parse(params);
      let expectedVersion = expectedVersionRef.current;
      if (expectedVersion === null) {
        const before = await readEvidence(expected.serviceId);
        if (!before.is_active) {
          throw new Error("This connection is already inactive.");
        }
        expectedVersion = before.state_version;
        expectedVersionRef.current = expectedVersion;
      }
      const rawResponse = await api.post<unknown>(
        "/assistant/actions/providers/connection-revoke",
        {
          actionRequestId,
          serviceId: expected.serviceId,
          expectedStateVersion: expectedVersion,
          confirmed: true,
        },
      );
      assertSecretFreeReadBack(rawResponse);
      const response = connectionEffectSchema.parse(rawResponse);
      setResultId(response.resource.serviceId);
      setReplayed(response.replayed);
      await verify(response.resource.serviceId, expectedVersion);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not revoke this connection."));
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
            {resultId ? "Connection revoked" : "Revoke service connection"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID cleared this connection's stored credential."
              : "This disables the legacy connection and securely clears its stored credential."}
          </DialogDescription>
        </DialogHeader>
        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Service</span>
              <Badge variant="secondary" className="max-w-[70%] truncate font-mono">
                {params.serviceId}
              </Badge>
            </div>
          </div>
        ) : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        {verified ? <p className="text-[11px] text-success">Revocation evidence verified.</p> : null}
        <DialogFooter>
          {!resultId ? (
            <>
              <Button type="button" variant="outline" onClick={close}>Cancel</Button>
              <Button type="button" variant="destructive" isLoading={submitting} disabled={submitting} onClick={() => void submit()}>
                Revoke connection
              </Button>
            </>
          ) : (
            <>
              {!verified && expectedVersionRef.current !== null ? (
                <Button type="button" variant="outline" isLoading={verifying} onClick={() => void verify(resultId, expectedVersionRef.current!)}>
                  <RefreshCw />
                  Retry verification
                </Button>
              ) : null}
              <Button type="button" variant="primary" disabled={!verified} onClick={() => { onComplete(resultId); close(); }}>
                {replayed ? "Report revoked connection" : "Done"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
