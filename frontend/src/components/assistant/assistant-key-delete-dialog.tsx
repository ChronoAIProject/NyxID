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
  keyDeleteActionParamsSchema,
} from "@/schemas/assistant-actions";

const keyResourceSchema = z
  .object({ keyId: actionControlIdentitySchema })
  .strict();
const assistantKeyDeleteResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

const authorizationEvidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    is_active: z.literal(true),
    state_version: z.number().int().positive(),
  })
  .passthrough();

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

export type AssistantKeyDeleteParams = z.infer<
  typeof keyDeleteActionParamsSchema
>;

export function AssistantKeyDeleteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantKeyDeleteParams;
  readonly onComplete: (keyId: string) => void;
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

  async function readLiveEvidence(keyId: string) {
    const value = await api.get<unknown>(
      `/api-keys/${encodeURIComponent(keyId)}/authorization`,
    );
    assertSecretFreeReadBack(value);
    const snapshot = authorizationEvidenceSchema.parse(value);
    if (snapshot.id !== keyId) {
      throw new Error("NyxID returned a different API key identity.");
    }
    return snapshot;
  }

  async function verifyDeleted(keyId: string): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      await api.get<unknown>(
        `/api-keys/${encodeURIComponent(keyId)}/authorization`,
      );
      throw new Error("NyxID still returned authorization evidence after delete.");
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 404) {
        setVerified(true);
        setError(null);
      } else if (
        caught instanceof Error &&
        caught.message.includes("still returned authorization evidence")
      ) {
        setError(caught.message);
      } else {
        setError(
          errorMessage(caught, "NyxID could not verify that this key was deleted."),
        );
      }
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function deleteKey() {
    if (submittingRef.current || resultKeyId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = keyDeleteActionParamsSchema.parse(params);
      const before = await readLiveEvidence(expected.keyId);
      const response = assistantKeyDeleteResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/keys/delete", {
          actionRequestId,
          keyId: expected.keyId,
          expectedStateVersion: before.state_version,
        }),
      );
      setResultKeyId(response.resource.keyId);
      setReplayed(response.replayed);
      await verifyDeleted(response.resource.keyId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not delete this key."));
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
            {resultKeyId ? "API key deleted" : "Delete API key"}
          </DialogTitle>
          <DialogDescription>
            {resultKeyId
              ? "NyxID retired this exact agent identity. The assistant receives only the safe key reference."
              : "This permanently retires the agent identity. NyxID asks you to confirm every time."}
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
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Deletion cannot be remembered or pre-approved. Confirm this exact
              key every time.
            </p>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        {verified ? (
          <p className="text-[11px] text-success">
            Authorization evidence is absent.
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
                variant="destructive"
                isLoading={submitting}
                disabled={submitting}
                onClick={() => void deleteKey()}
              >
                Delete key
              </Button>
            </>
          ) : (
            <>
              {!verified ? (
                <Button
                  type="button"
                  variant="outline"
                  isLoading={verifying}
                  onClick={() => void verifyDeleted(resultKeyId)}
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
                  onComplete(resultKeyId);
                  close();
                }}
              >
                {replayed ? "Report deleted key" : "Done"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
