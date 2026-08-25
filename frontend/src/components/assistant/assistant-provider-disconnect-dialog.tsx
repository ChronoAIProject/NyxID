import { useRef, useState } from "react";
import { RefreshCw, ShieldAlert } from "lucide-react";
import { z } from "zod";
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
import {
  assertSecretFreeReadBack,
  errorMessage,
} from "@/components/assistant/assistant-action-dialog-utils";
import { api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  providerDisconnectActionParamsSchema,
} from "@/schemas/assistant-actions";
import { assistantOneTimeMaterialSchema } from "@/schemas/assistant-action-effects";

const providerEvidenceSchema = z
  .object({
    provider_id: actionControlIdentitySchema,
    status: z.string().min(1),
    state_version: z.number().int().nonnegative(),
    updated_at: z.string(),
  })
  .strict();
const providerEffectSchema = z
  .object({
    resource: z
      .object({ providerId: actionControlIdentitySchema })
      .strict(),
    replayed: z.boolean(),
    oneTimeMaterial: assistantOneTimeMaterialSchema,
  })
  .strict();

export type AssistantProviderDisconnectParams = z.infer<
  typeof providerDisconnectActionParamsSchema
>;

export function AssistantProviderDisconnectDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantProviderDisconnectParams;
  readonly onComplete: (providerId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verifyingRef = useRef(false);
  const expectedVersionRef = useRef<number | null>(null);
  const [confirmed, setConfirmed] = useState(false);
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
    setConfirmed(false);
    setSubmitting(false);
    setVerifying(false);
    setVerified(false);
    setResultId(null);
    setReplayed(false);
    setError(null);
    onOpenChange(false);
  }

  async function readEvidence(providerId: string) {
    const value = await api.get<unknown>(
      `/providers/${encodeURIComponent(providerId)}/authorization`,
    );
    assertSecretFreeReadBack(value);
    const evidence = providerEvidenceSchema.parse(value);
    if (evidence.provider_id !== providerId) {
      throw new Error("NyxID returned a different provider identity.");
    }
    return evidence;
  }

  async function verify(providerId: string, expectedVersion: number) {
    if (verifyingRef.current) return;
    verifyingRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const evidence = await readEvidence(providerId);
      if (
        evidence.status !== "revoked" ||
        evidence.state_version !== expectedVersion + 1
      ) {
        throw new Error("NyxID provider evidence did not prove the disconnect.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not verify this disconnect."));
    } finally {
      verifyingRef.current = false;
      setVerifying(false);
    }
  }

  async function submit() {
    if (submittingRef.current || resultId || !confirmed) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = providerDisconnectActionParamsSchema.parse(params);
      let expectedVersion = expectedVersionRef.current;
      if (expectedVersion === null) {
        const before = await readEvidence(expected.providerId);
        if (before.status === "revoked") {
          throw new Error("This provider is already disconnected.");
        }
        expectedVersion = before.state_version;
        expectedVersionRef.current = expectedVersion;
      }
      const rawResponse = await api.post<unknown>(
        "/assistant/actions/providers/provider-disconnect",
        {
          actionRequestId,
          providerId: expected.providerId,
          expectedStateVersion: expectedVersion,
          confirmed,
        },
      );
      assertSecretFreeReadBack(rawResponse);
      const response = providerEffectSchema.parse(rawResponse);
      setResultId(response.resource.providerId);
      setReplayed(response.replayed);
      await verify(response.resource.providerId, expectedVersion);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not disconnect this provider."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{resultId ? "Provider disconnected" : "Disconnect provider"}</DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID revoked this provider token and cleared its stored token material."
              : "This revokes the legacy provider token and securely clears its stored token material."}
          </DialogDescription>
        </DialogHeader>
        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Provider</span>
              <Badge variant="secondary" className="max-w-[70%] truncate font-mono">
                {params.providerId}
              </Badge>
            </div>
            <label className="flex items-start gap-2">
              <Checkbox
                checked={confirmed}
                onCheckedChange={(value) => setConfirmed(value === true)}
              />
              <span className="flex items-start gap-1.5 text-destructive">
                <ShieldAlert className="mt-0.5 size-3 shrink-0" />I understand
                this revokes and clears the legacy provider token.
              </span>
            </label>
          </div>
        ) : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        {verified ? <p className="text-[11px] text-success">Disconnect evidence verified.</p> : null}
        <DialogFooter>
          {!resultId ? (
            <>
              <Button type="button" variant="outline" onClick={close}>Cancel</Button>
              <Button type="button" variant="destructive" isLoading={submitting} disabled={submitting || !confirmed} onClick={() => void submit()}>
                Disconnect provider
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
                {replayed ? "Report disconnected provider" : "Done"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
