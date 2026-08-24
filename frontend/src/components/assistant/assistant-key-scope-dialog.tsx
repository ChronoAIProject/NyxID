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
  keyExtendScopeActionParamsSchema,
} from "@/schemas/assistant-actions";

const keyResourceSchema = z
  .object({ keyId: actionControlIdentitySchema })
  .strict();
const assistantKeyExtendResponseSchema = z
  .object({
    resource: keyResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

const authorizationEvidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    is_active: z.literal(true),
    allowed_service_ids: z.array(actionControlIdentitySchema).max(64),
    allow_all_services: z.literal(false),
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

export type AssistantKeyScopeParams = z.infer<
  typeof keyExtendScopeActionParamsSchema
>;

export function AssistantKeyScopeDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantKeyScopeParams;
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

  async function readEvidence(keyId: string) {
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

  async function verifyScope(
    keyId: string,
    expected: AssistantKeyScopeParams,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const snapshot = await readEvidence(keyId);
      const held = new Set(snapshot.allowed_service_ids);
      const missing = expected.addServiceIds.filter((id) => !held.has(id));
      if (missing.length > 0 || snapshot.allow_all_services) {
        throw new Error("NyxID key verification did not match this action.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify the widened key scope."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function extendScope() {
    if (submittingRef.current || resultKeyId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = keyExtendScopeActionParamsSchema.parse(params);
      const before = await readEvidence(expected.keyId);
      const response = assistantKeyExtendResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/keys/extend-scope", {
          actionRequestId,
          keyId: expected.keyId,
          addServiceIds: [...expected.addServiceIds],
          expectedStateVersion: before.state_version,
        }),
      );
      setResultKeyId(response.resource.keyId);
      setReplayed(response.replayed);
      await verifyScope(response.resource.keyId, expected);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not extend this key's scope."));
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
            {resultKeyId ? "API key scope extended" : "Extend API key scope"}
          </DialogTitle>
          <DialogDescription>
            {resultKeyId
              ? "NyxID widened this key's allowed services. The assistant receives only the safe key reference."
              : "Widening what this agent can reach is confirmed every time and is never remembered."}
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
            <div className="space-y-2">
              <span className="text-muted-foreground">Add services</span>
              <div className="flex flex-wrap gap-1.5">
                {params.addServiceIds.map((serviceId) => (
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
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        {verified ? (
          <p className="text-[11px] text-success">
            Exact widened service set verified.
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
                onClick={() => void extendScope()}
              >
                Extend scope
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
                      keyExtendScopeActionParamsSchema.parse(params);
                    void verifyScope(resultKeyId, expected);
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
                  onComplete(resultKeyId);
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
