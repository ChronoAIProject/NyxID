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
import {
  actionControlIdentitySchema,
  keyCreateActionParamsSchema,
} from "@/schemas/assistant-actions";
import { copyToClipboard } from "@/lib/utils";

const keyResourceSchema = z
  .object({ keyId: actionControlIdentitySchema })
  .strict();
const createdKeyResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.literal(false),
    fullKey: z.string().min(1).max(4_096),
  })
  .strict();
const replayedKeyResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.literal(true),
  })
  .strict();
const assistantKeyCreateResponseSchema = z.discriminatedUnion("replayed", [
  createdKeyResponseSchema,
  replayedKeyResponseSchema,
]);

const serviceSnapshotSchema = z
  .object({
    id: actionControlIdentitySchema,
    is_active: z.literal(true),
    credential_source: z.object({ type: z.literal("personal") }).passthrough(),
  })
  .passthrough();
const apiKeySnapshotSchema = z
  .object({
    id: actionControlIdentitySchema,
    name: z.string().min(1).max(200),
    platform: z.string().min(1).max(100),
    scopes: z.literal("proxy"),
    is_active: z.literal(true),
    allowed_service_ids: z.array(actionControlIdentitySchema).min(1).max(64),
    allowed_node_ids: z.array(actionControlIdentitySchema).length(0),
    allow_all_services: z.literal(false),
    allow_all_nodes: z.literal(false),
  })
  .passthrough();

type AssistantKeyEffectResponse = z.infer<
  typeof assistantKeyCreateResponseSchema
>;

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

function sameStringSet(left: readonly string[], right: readonly string[]) {
  if (left.length !== right.length) return false;
  const sortedLeft = [...left].sort();
  const sortedRight = [...right].sort();
  return sortedLeft.every((value, index) => value === sortedRight[index]);
}

function errorMessage(caught: unknown, fallback: string): string {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof Error && caught.message.trim()) return caught.message;
  return fallback;
}

function KeyEffectResult({
  result,
  verified,
  verifying,
  onError,
  onRetryVerification,
  onFinish,
}: {
  readonly result: AssistantKeyEffectResponse;
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
        "The browser could not copy the key. Select and copy it manually.",
      );
    }
  }

  return (
    <>
      {result.replayed ? (
        <div className="flex items-start gap-3 border-y border-border py-4">
          <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 space-y-1">
            <p className="text-[13px] font-medium">Existing key</p>
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
              title="Copy API key"
              aria-label="Copy API key"
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
            <span>I saved this key in a secure location.</span>
          </label>
        </div>
      )}
      {verified ? (
        <p className="text-[11px] text-success">
          Exact least-scope access verified.
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
          {result.replayed ? "Report existing key" : "I have saved it"}
        </Button>
      </DialogFooter>
    </>
  );
}

export interface AssistantKeyCreateParams {
  readonly name: string;
  readonly platform: string;
  readonly allowedServiceIds: readonly string[];
}

export function AssistantKeyCreateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantKeyCreateParams;
  readonly onComplete: (keyId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [result, setResult] = useState<AssistantKeyEffectResponse | null>(null);
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

  async function verifyAllowedServices(
    allowedServiceIds: readonly string[],
  ): Promise<void> {
    const snapshots = await Promise.all(
      allowedServiceIds.map(async (serviceId) => {
        const value = await api.get<unknown>(
          `/keys/${encodeURIComponent(serviceId)}`,
        );
        assertSecretFreeReadBack(value);
        return serviceSnapshotSchema.parse(value);
      }),
    );
    for (const [index, snapshot] of snapshots.entries()) {
      if (snapshot.id !== allowedServiceIds[index]) {
        throw new Error("NyxID returned a different service identity.");
      }
    }
  }

  async function verifyCreatedKey(
    effect: AssistantKeyEffectResponse,
    expected: z.infer<typeof keyCreateActionParamsSchema>,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const value = await api.get<unknown>(
        `/api-keys/${encodeURIComponent(effect.resource.keyId)}`,
      );
      assertSecretFreeReadBack(value);
      const snapshot = apiKeySnapshotSchema.parse(value);
      if (
        snapshot.id !== effect.resource.keyId ||
        snapshot.name !== expected.name ||
        snapshot.platform !== expected.platform ||
        !sameStringSet(snapshot.allowed_service_ids, expected.allowedServiceIds)
      ) {
        throw new Error("NyxID key verification did not match this action.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify the created API key."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function createKey() {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = keyCreateActionParamsSchema.parse(params);
      await verifyAllowedServices(expected.allowedServiceIds);
      const response = assistantKeyCreateResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/key-create", {
          actionRequestId,
          name: expected.name,
          platform: expected.platform,
          allowedServiceIds: [...expected.allowedServiceIds],
        }),
      );
      setResult(response);
      await verifyCreatedKey(response, expected);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not create this key."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  async function retryVerification() {
    if (!result) return;
    try {
      const expected = keyCreateActionParamsSchema.parse(params);
      await verifyCreatedKey(result, expected);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify the created API key."),
      );
    }
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
            {result?.replayed ? "API key already created" : "Create API key"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? result.replayed
                ? "The original one-time secret is no longer available."
                : "This key is shown only once. Copy and store it securely now."
              : "Confirm the exact key identity and service boundary."}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Name</span>
              <span className="min-w-0 truncate font-medium">
                {params.name}
              </span>
            </div>
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Platform</span>
              <Badge variant="secondary" className="max-w-[70%] truncate">
                {params.platform}
              </Badge>
            </div>
            <div className="space-y-2">
              <span className="text-muted-foreground">Allowed services</span>
              <div className="flex flex-wrap gap-1.5">
                {params.allowedServiceIds.map((serviceId) => (
                  <Badge
                    key={serviceId}
                    variant="secondary"
                    className="max-w-full truncate font-mono"
                  >
                    {serviceId}
                  </Badge>
                ))}
              </div>
            </div>
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              Proxy access only. All services outside this list and every node
              are denied.
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
              onClick={() => void createKey()}
            >
              Create key
            </Button>
          </DialogFooter>
        ) : null}
        {result ? (
          <KeyEffectResult
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
