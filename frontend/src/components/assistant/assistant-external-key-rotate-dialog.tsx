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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError, api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  externalKeyRotateActionParamsSchema,
} from "@/schemas/assistant-actions";

const externalKeyResourceSchema = z
  .object({ externalKeyId: actionControlIdentitySchema })
  .strict();
const assistantExternalKeyRotateResponseSchema = z
  .object({
    resource: externalKeyResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

const PINNED_CREDENTIAL_TYPES = [
  "api_key",
  "oauth2",
  "bearer",
  "basic",
  "ssh_certificate",
  "node_managed",
  "gcp_service_account",
] as const;
const PINNED_STATUSES = [
  "active",
  "expired",
  "revoked",
  "failed",
  "refresh_failed",
  "pending_auth",
] as const;

const externalKeyEvidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    credential_type: z.enum(PINNED_CREDENTIAL_TYPES),
    status: z.enum(PINNED_STATUSES),
    expires_at: z.string().datetime({ offset: true }).nullable(),
    last_used_at: z.string().datetime({ offset: true }).nullable(),
    updated_at: z.string().datetime({ offset: true }),
  })
  .strict();

const FORBIDDEN_READ_BACK_FIELDS = new Set([
  "apikey",
  "fullkey",
  "keyhash",
  "credential",
  "credentials",
  "accesstoken",
  "refreshtoken",
  "authorization",
  "cookie",
  "cookies",
  "secret",
  "secrets",
  "clientsecret",
  "password",
  "token",
  "passphrase",
  "usercode",
  "devicecode",
  "rawbody",
]);
const SECRET_READ_BACK_VALUE =
  /(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})/i;

function assertSecretFreeReadBack(value: unknown): void {
  if (typeof value === "string" && SECRET_READ_BACK_VALUE.test(value)) {
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

export interface AssistantExternalKeyRotateParams {
  readonly externalKeyId: string;
}

export function AssistantExternalKeyRotateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantExternalKeyRotateParams;
  readonly onComplete: (externalKeyId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [credential, setCredential] = useState("");
  const [result, setResult] = useState<z.infer<
    typeof assistantExternalKeyRotateResponseSchema
  > | null>(null);
  const [error, setError] = useState<string | null>(null);

  function close() {
    setResult(null);
    setError(null);
    setVerified(false);
    setCredential("");
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function verifyRotation(externalKeyId: string): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const value = await api.get<unknown>(
        `/api-keys/external/${encodeURIComponent(externalKeyId)}/authorization`,
      );
      assertSecretFreeReadBack(value);
      const evidence = externalKeyEvidenceSchema.parse(value);
      if (evidence.id !== externalKeyId) {
        throw new Error("NyxID returned a different external key identity.");
      }
      if ("label" in (value as object) || "error_message" in (value as object)) {
        throw new Error("NyxID returned secret-bearing verification data.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(
          caught,
          "NyxID could not verify the rotated external credential.",
        ),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function rotateKey() {
    if (submittingRef.current || result) return;
    if (!credential.trim()) {
      setError("Enter the replacement credential.");
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = externalKeyRotateActionParamsSchema.parse(params);
      const response = assistantExternalKeyRotateResponseSchema.parse(
        await api.post<unknown>(
          "/assistant/actions/endpoints/external-key-rotate",
          {
            actionRequestId,
            externalKeyId: expected.externalKeyId,
            credential,
          },
        ),
      );
      setResult(response);
      setCredential("");
      await verifyRotation(response.resource.externalKeyId);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not rotate this external credential."),
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
            {result?.replayed
              ? "External credential already rotated"
              : "Rotate external credential"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? "NyxID verified the stored credential through its authorization evidence. The replacement secret never leaves this browser."
              : "Paste the replacement secret here. NyxID stores it; the assistant receives only the external-key reference."}
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
            <div className="space-y-1.5">
              <Label htmlFor="assistant-external-key-rotate-secret">
                Replacement credential
              </Label>
              <Input
                id="assistant-external-key-rotate-secret"
                type="password"
                autoComplete="off"
                value={credential}
                onChange={(event) => setCredential(event.target.value)}
              />
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
            External credential rotation verified.
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
                variant="primary"
                isLoading={submitting}
                disabled={submitting || !credential.trim()}
                onClick={() => void rotateKey()}
              >
                Rotate credential
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
                    void verifyRotation(result.resource.externalKeyId)
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
                Report rotation
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
