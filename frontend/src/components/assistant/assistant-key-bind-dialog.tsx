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
  keyBindCredentialActionParamsSchema,
} from "@/schemas/assistant-actions";

const keyResourceSchema = z
  .object({ keyId: actionControlIdentitySchema })
  .strict();
const assistantKeyBindResponseSchema = z
  .object({
    resource: keyResourceSchema,
    bindingId: actionControlIdentitySchema,
    replayed: z.boolean(),
  })
  .strict();

const bindingEvidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    api_key_id: actionControlIdentitySchema,
    user_service_id: actionControlIdentitySchema,
    user_api_key_id: actionControlIdentitySchema,
    created_at: z.string().datetime({ offset: true }),
    updated_at: z.string().datetime({ offset: true }),
  })
  .strict();

const SECRET_VALUE =
  /(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})/i;
const FORBIDDEN_READ_BACK_FIELDS = new Set([
  "accesstoken",
  "apikey",
  "authorization",
  "cookie",
  "cookies",
  "credential",
  "credentials",
  "fullkey",
  "keyhash",
  "password",
  "rawbody",
  "rawupstreambody",
  "refreshtoken",
  "secret",
  "secrets",
  "token",
]);

function assertSecretFreeReadBack(value: unknown): void {
  if (typeof value === "string" && SECRET_VALUE.test(value)) {
    throw new Error("NyxID returned secret-bearing verification data.");
  }
  if (Array.isArray(value)) {
    for (const entry of value) assertSecretFreeReadBack(entry);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, entry] of Object.entries(value)) {
    const normalized = key
      .replace(/[^A-Za-z0-9]/g, "")
      .toLocaleLowerCase("en-US");
    if (FORBIDDEN_READ_BACK_FIELDS.has(normalized)) {
      throw new Error("NyxID returned secret-bearing verification data.");
    }
    assertSecretFreeReadBack(entry);
  }
}

function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

export type AssistantKeyBindParams = z.infer<
  typeof keyBindCredentialActionParamsSchema
>;

export function AssistantKeyBindDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantKeyBindParams;
  readonly onComplete: (resource: {
    readonly keyId: string;
    readonly userServiceId: string;
  }) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [resultKeyId, setResultKeyId] = useState<string | null>(null);
  const [replayed, setReplayed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setResultKeyId(null);
    setError(null);
    setVerified(false);
    setReplayed(false);
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function verifyBinding(
    keyId: string,
    expected: AssistantKeyBindParams,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const value = await api.get<unknown>(
        `/api-keys/${encodeURIComponent(keyId)}/bindings/by-service/${encodeURIComponent(expected.userServiceId)}/authorization`,
      );
      assertSecretFreeReadBack(value);
      const snapshot = bindingEvidenceSchema.parse(value);
      if (
        snapshot.api_key_id !== expected.keyId ||
        snapshot.user_service_id !== expected.userServiceId ||
        snapshot.user_api_key_id !== expected.externalKeyId
      ) {
        throw new Error("NyxID binding verification did not match this action.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify this credential binding."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function bindCredential() {
    if (submittingRef.current || resultKeyId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = keyBindCredentialActionParamsSchema.parse(params);
      const response = assistantKeyBindResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/keys/bind-credential", {
          actionRequestId,
          keyId: expected.keyId,
          userServiceId: expected.userServiceId,
          externalKeyId: expected.externalKeyId,
        }),
      );
      setResultKeyId(response.resource.keyId);
      setReplayed(response.replayed);
      await verifyBinding(response.resource.keyId, expected);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not bind this credential."),
      );
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
            {resultKeyId ? "Credential bound" : "Bind credential"}
          </DialogTitle>
          <DialogDescription>
            {resultKeyId
              ? "NyxID bound this agent key to the chosen credential. The assistant receives only the safe key reference."
              : "Binding a dedicated credential widens this agent's reach and is never remembered."}
          </DialogDescription>
        </DialogHeader>

        {!resultKeyId ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Key</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.keyId}
              </Badge>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Service</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.userServiceId}
              </Badge>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Credential</span>
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
            Exact binding evidence verified.
          </p>
        ) : null}

        <DialogFooter>
          {!resultKeyId ? (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="primary"
                isLoading={submitting}
                disabled={submitting}
                onClick={() => void bindCredential()}
              >
                Bind credential
              </Button>
            </>
          ) : (
            <>
              {!verified ? (
                <Button
                  type="button"
                  variant="outline"
                  isLoading={verifying}
                  onClick={() => {
                    const expected =
                      keyBindCredentialActionParamsSchema.parse(params);
                    void verifyBinding(resultKeyId, expected);
                  }}
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
                  onComplete({
                    keyId: resultKeyId,
                    userServiceId: params.userServiceId,
                  });
                  close();
                }}
              >
                {replayed ? "Report existing key" : "Done"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
