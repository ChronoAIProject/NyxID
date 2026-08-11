import { useRef, useState } from "react";
import { Check, Copy, KeyRound, RefreshCw } from "lucide-react";
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
import { ApiError, api } from "@/lib/api-client";
import { copyToClipboard } from "@/lib/utils";
import {
  actionControlIdentitySchema,
  keyRotateActionParamsSchema,
} from "@/schemas/assistant-actions";

const authoritativeTimestampSchema = z.string().datetime({ offset: true });
const keyResourceSchema = z
  .object({ keyId: actionControlIdentitySchema })
  .strict();
const createdRotationResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.literal(false),
    requestedAt: authoritativeTimestampSchema,
    fullKey: z.string().min(1).max(4_096),
  })
  .strict();
const replayedRotationResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.literal(true),
    requestedAt: authoritativeTimestampSchema,
  })
  .strict();
const assistantKeyRotateResponseSchema = z.discriminatedUnion("replayed", [
  createdRotationResponseSchema,
  replayedRotationResponseSchema,
]);

const apiKeySnapshotSchema = z
  .object({
    id: actionControlIdentitySchema,
    is_active: z.literal(true),
    created_at: authoritativeTimestampSchema,
    rotation_predecessor_id: actionControlIdentitySchema,
    state_version: z.number().int().positive(),
    updated_at: authoritativeTimestampSchema,
  })
  .passthrough();

type AssistantKeyRotateEffectResponse = z.infer<
  typeof assistantKeyRotateResponseSchema
>;
type ApiKeySnapshot = z.infer<typeof apiKeySnapshotSchema>;

const FORBIDDEN_READ_BACK_FIELDS = new Set([
  "accesstoken",
  "authorization",
  "cookie",
  "fullkey",
  "keyhash",
  "rawbody",
  "rawupstreambody",
  "refreshtoken",
  "secret",
]);

function assertSecretFreeReadBack(value: unknown): void {
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

function KeyRotationResult({
  result,
  verified,
  verifying,
  onError,
  onRetryVerification,
  onFinish,
}: {
  readonly result: AssistantKeyRotateEffectResponse;
  readonly verified: boolean;
  readonly verifying: boolean;
  readonly onError: (message: string) => void;
  readonly onRetryVerification: () => void;
  readonly onFinish: (keyId: string) => void;
}) {
  const [saved, setSaved] = useState(false);
  const [copied, setCopied] = useState(false);

  async function copySecret() {
    if (result.replayed) return;
    try {
      await copyToClipboard(result.fullKey);
      setCopied(true);
    } catch {
      onError(
        "The browser could not copy the replacement key. Select and copy it manually.",
      );
    }
  }

  return (
    <>
      {result.replayed ? (
        <div className="flex items-start gap-3 border-y border-border py-4">
          <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 space-y-1">
            <p className="text-[13px] font-medium">Existing replacement key</p>
            <p className="break-all font-mono text-[12px] text-muted-foreground">
              {result.resource.keyId}
            </p>
          </div>
        </div>
      ) : (
        <div className="space-y-4 border-y border-border py-4">
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 select-all break-all rounded-lg border border-border bg-muted px-3 py-2 font-mono text-[12px]">
              {result.fullKey}
            </code>
            <Button
              type="button"
              variant="outline"
              size="icon"
              title="Copy replacement API key"
              aria-label="Copy replacement API key"
              onClick={() => void copySecret()}
            >
              {copied ? <Check className="text-success" /> : <Copy />}
            </Button>
          </div>
          <label className="flex cursor-pointer items-start gap-2 text-[12px]">
            <Checkbox
              checked={saved}
              onCheckedChange={(value) => setSaved(value === true)}
            />
            <span>I saved this replacement key in a secure location.</span>
          </label>
        </div>
      )}
      {verified ? (
        <p className="text-[11px] text-success">
          Exact rotation lineage verified.
        </p>
      ) : null}
      <DialogFooter>
        {!verified ? (
          <Button
            type="button"
            variant="outline"
            isLoading={verifying}
            onClick={onRetryVerification}
          >
            <RefreshCw />
            Retry verification
          </Button>
        ) : null}
        <Button
          type="button"
          variant="primary"
          disabled={!verified || (!result.replayed && !saved)}
          onClick={() => onFinish(result.resource.keyId)}
        >
          {result.replayed ? "Report replacement key" : "I have saved it"}
        </Button>
      </DialogFooter>
    </>
  );
}

export interface AssistantKeyRotateParams {
  readonly keyId: string;
}

export function AssistantKeyRotateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantKeyRotateParams;
  readonly onComplete: (keyId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [result, setResult] = useState<AssistantKeyRotateEffectResponse | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);

  const oneTimeSecret = result && !result.replayed ? result.fullKey : null;

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

  async function readKey(keyId: string): Promise<ApiKeySnapshot> {
    const value = await api.get<unknown>(
      `/api-keys/${encodeURIComponent(keyId)}`,
    );
    assertSecretFreeReadBack(value);
    const snapshot = apiKeySnapshotSchema.parse(value);
    if (snapshot.id !== keyId) {
      throw new Error("NyxID returned a different API key identity.");
    }
    return snapshot;
  }

  async function verifyReplacement(
    effect: AssistantKeyRotateEffectResponse,
    expectedPredecessorId: string,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const replacement = await readKey(effect.resource.keyId);
      const requestedAt = Date.parse(effect.requestedAt);
      const createdAt = Date.parse(replacement.created_at);
      const updatedAt = Date.parse(replacement.updated_at);
      if (
        replacement.id === expectedPredecessorId ||
        replacement.rotation_predecessor_id !== expectedPredecessorId ||
        createdAt < requestedAt ||
        updatedAt < requestedAt ||
        updatedAt < createdAt
      ) {
        throw new Error("NyxID key rotation did not match this action.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify the replacement key."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function rotateKey() {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = keyRotateActionParamsSchema.parse(params);
      const response = assistantKeyRotateResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/key-rotate", {
          actionRequestId,
          keyId: expected.keyId,
        }),
      );
      setResult(response);
      await verifyReplacement(response, expected.keyId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not rotate this key."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  async function retryVerification() {
    if (!result) return;
    const expected = keyRotateActionParamsSchema.parse(params);
    await verifyReplacement(result, expected.keyId);
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !oneTimeSecret) close();
      }}
    >
      <DialogContent
        onEscapeKeyDown={(event) => {
          if (oneTimeSecret) event.preventDefault();
        }}
        onInteractOutside={(event) => {
          if (oneTimeSecret) event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>
            {result?.replayed ? "API key already rotated" : "Rotate API key"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? result.replayed
                ? "The original one-time replacement secret is no longer available."
                : "This replacement key is shown only once. Copy and store it securely now."
              : "Confirm the exact predecessor before replacing its credential."}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Predecessor</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.keyId}
              </Badge>
            </div>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              NyxID preserves the key's authority and disables this exact
              predecessor atomically.
            </p>
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        {!result ? (
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              Cancel
            </Button>
            <Button
              type="button"
              variant="primary"
              isLoading={submitting}
              disabled={submitting}
              onClick={() => void rotateKey()}
            >
              Rotate key
            </Button>
          </DialogFooter>
        ) : null}
        {result ? (
          <KeyRotationResult
            result={result}
            verified={verified}
            verifying={verifying}
            onError={setError}
            onRetryVerification={() => void retryVerification()}
            onFinish={(keyId) => {
              onComplete(keyId);
              close();
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
