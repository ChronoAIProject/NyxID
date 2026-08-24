import { useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  assertSecretFreeReadBack,
  errorMessage,
} from "@/components/assistant/assistant-action-dialog-utils";
import { api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  providerSetAppCredentialsActionParamsSchema,
} from "@/schemas/assistant-actions";
import { assistantOneTimeMaterialSchema } from "@/schemas/assistant-action-effects";

const credentialsEvidenceSchema = z
  .object({
    provider_id: actionControlIdentitySchema,
    has_credentials: z.boolean(),
    state_version: z.number().int().nonnegative(),
    updated_at: z.string().nullable(),
  })
  .strict();
const credentialsEffectSchema = z
  .object({
    resource: z
      .object({ providerId: actionControlIdentitySchema })
      .strict(),
    replayed: z.boolean(),
    oneTimeMaterial: assistantOneTimeMaterialSchema,
  })
  .strict();

interface CredentialFormValues {
  readonly clientId: string;
  readonly clientSecret: string;
}

export type AssistantProviderSetAppCredentialsParams = z.infer<
  typeof providerSetAppCredentialsActionParamsSchema
>;

export function AssistantProviderSetAppCredentialsDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantProviderSetAppCredentialsParams;
  readonly onComplete: (providerId: string) => void;
}) {
  const form = useAppForm<CredentialFormValues>({
    defaultValues: { clientId: "", clientSecret: "" },
  });
  const submittingRef = useRef(false);
  const verifyingRef = useRef(false);
  const expectedVersionRef = useRef<number | null>(null);
  const submittedSecretRef = useRef<string | null>(null);
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
    submittedSecretRef.current = null;
    form.reset({ clientId: "", clientSecret: "" });
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
      `/providers/${encodeURIComponent(providerId)}/credentials/authorization`,
    );
    assertSecretFreeReadBack(value);
    const evidence = credentialsEvidenceSchema.parse(value);
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
        !evidence.has_credentials ||
        evidence.state_version !== expectedVersion + 1
      ) {
        throw new Error(
          "NyxID provider evidence did not prove the credential update.",
        );
      }
      submittedSecretRef.current = null;
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify these app credentials."),
      );
    } finally {
      verifyingRef.current = false;
      setVerifying(false);
    }
  }

  async function submit(values: CredentialFormValues) {
    if (submittingRef.current || resultId) return;
    const clientId = values.clientId;
    if (!clientId.trim()) {
      setError("Enter the OAuth client ID.");
      return;
    }
    const clientSecret = submittedSecretRef.current ?? values.clientSecret;
    submittedSecretRef.current = clientSecret;
    form.setValue("clientSecret", "", {
      shouldDirty: false,
      shouldTouch: false,
      shouldValidate: false,
    });
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = providerSetAppCredentialsActionParamsSchema.parse(params);
      let expectedVersion = expectedVersionRef.current;
      if (expectedVersion === null) {
        const before = await readEvidence(expected.providerId);
        expectedVersion = before.state_version;
        expectedVersionRef.current = expectedVersion;
      }
      const body: {
        actionRequestId: string;
        providerId: string;
        clientId: string;
        clientSecret?: string;
        expectedStateVersion: number;
      } = {
        actionRequestId,
        providerId: expected.providerId,
        clientId,
        expectedStateVersion: expectedVersion,
      };
      if (clientSecret) body.clientSecret = clientSecret;
      const rawResponse = await api.post<unknown>(
        "/assistant/actions/providers/set-app-credentials",
        body,
      );
      assertSecretFreeReadBack(rawResponse);
      const response = credentialsEffectSchema.parse(rawResponse);
      setResultId(response.resource.providerId);
      setReplayed(response.replayed);
      await verify(response.resource.providerId, expectedVersion);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not save these app credentials."));
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
            {resultId ? "OAuth app credentials saved" : "Set OAuth app credentials"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID stored the credentials and returned only the provider reference."
              : "Enter your OAuth app credentials here. They are never returned to the assistant."}
          </DialogDescription>
        </DialogHeader>
        {!resultId ? (
          <form
            id="assistant-provider-credentials-form"
            className="space-y-4 border-y border-border py-4"
            onSubmit={form.handleSubmit((values) => void submit(values))}
          >
            <div className="space-y-2">
              <Label htmlFor="assistant-provider-client-id">Client ID</Label>
              <Input
                id="assistant-provider-client-id"
                autoComplete="off"
                disabled={submitting}
                {...form.register("clientId", { required: true, maxLength: 500 })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="assistant-provider-client-secret">Client secret</Label>
              <Input
                id="assistant-provider-client-secret"
                type="password"
                autoComplete="off"
                disabled={submitting}
                {...form.register("clientSecret", { maxLength: 2000 })}
              />
            </div>
          </form>
        ) : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        {verified ? <p className="text-[11px] text-success">Credential evidence verified.</p> : null}
        <DialogFooter>
          {!resultId ? (
            <>
              <Button type="button" variant="outline" onClick={close}>Cancel</Button>
              <Button type="submit" form="assistant-provider-credentials-form" variant="primary" isLoading={submitting} disabled={submitting}>
                Save credentials
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
                {replayed ? "Report saved credentials" : "Done"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
