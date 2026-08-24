import { useRef, useState } from "react";
import { RefreshCw, Server } from "lucide-react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ApiError, api } from "@/lib/api-client";
import {
  actionControlIdentitySchema,
  serviceRotateCredentialActionParamsSchema,
} from "@/schemas/assistant-actions";

const serviceResourceSchema = z
  .object({ userServiceId: actionControlIdentitySchema })
  .strict();
const assistantServiceRotateResponseSchema = z
  .object({
    resource: serviceResourceSchema,
    replayed: z.boolean(),
  })
  .strict()
  .superRefine((value, context) => {
    const record = value as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (
        /credential|secret|token|fullkey|password/i.test(
          key.replace(/[^A-Za-z0-9]/g, ""),
        )
      ) {
        context.addIssue({
          code: "custom",
          message: "NyxID returned credential material from a rotate effect.",
        });
      }
    }
  });

const authorizationEvidenceSchema = z
  .object({
    id: z.string().min(1),
    api_key_id: z.string().min(1).nullish(),
    is_active: z.boolean(),
    status: z.string().min(1),
    connection_status: z.string().nullable(),
    granted_scopes: z.array(z.string()).nullable(),
    last_authorized_at: z.string().nullable(),
    node_id: z.string().nullable(),
    rotation_predecessor_id: z.string().nullable().optional(),
    state_version: z.number().int().positive().optional(),
    updated_at: z.string().optional(),
  })
  .strict();
type AuthorizationEvidence = z.infer<typeof authorizationEvidenceSchema>;

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
  "rawupstreambody",
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

export interface AssistantServiceRotateCredentialParams {
  readonly userServiceId: string;
}

export function AssistantServiceRotateCredentialDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantServiceRotateCredentialParams;
  readonly onComplete: (userServiceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [credential, setCredential] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);
  const [expectedStateVersion, setExpectedStateVersion] = useState<number | null>(null);
  const [expectedPredecessorId, setExpectedPredecessorId] = useState<string | null>(null);

  function close() {
    setError(null);
    setVerified(false);
    setResultId(null);
    setExpectedStateVersion(null);
    setExpectedPredecessorId(null);
    setCredential("");
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function readEvidence(userServiceId: string): Promise<AuthorizationEvidence> {
    const value = await api.get<unknown>(
      `/keys/${encodeURIComponent(userServiceId)}/authorization`,
    );
    assertSecretFreeReadBack(value);
    return authorizationEvidenceSchema.parse(value);
  }

  async function verifyEvidence(
    userServiceId: string,
    expectedVersion: number,
    predecessorId: string | null,
    requireAdvance: boolean,
    previousUpdatedAt?: string,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const evidence = await readEvidence(userServiceId);
      if (evidence.id !== userServiceId) {
        throw new Error("NyxID returned a different service identity.");
      }
      if (evidence.state_version !== expectedVersion) {
        throw new Error("NyxID service evidence did not show the expected state advance.");
      }
      if (!evidence.rotation_predecessor_id) {
        throw new Error("NyxID rotation lineage was missing.");
      }
      if (predecessorId && evidence.rotation_predecessor_id !== predecessorId) {
        throw new Error("NyxID rotation lineage did not identify the replaced credential.");
      }
      if (requireAdvance) {
        if (!evidence.updated_at || !previousUpdatedAt) {
          throw new Error("NyxID rotation timestamp evidence was missing.");
        }
        const beforeTimestamp = Date.parse(previousUpdatedAt);
        const afterTimestamp = Date.parse(evidence.updated_at);
        if (
          !Number.isFinite(beforeTimestamp) ||
          !Number.isFinite(afterTimestamp) ||
          afterTimestamp <= beforeTimestamp
        ) {
          throw new Error("NyxID rotation timestamp did not advance.");
        }
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify this credential rotation."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function submit() {
    if (submittingRef.current || resultId) return;
    if (!credential.trim()) {
      setError("Enter the replacement credential.");
      return;
    }
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = serviceRotateCredentialActionParamsSchema.parse({
        userServiceId: params.userServiceId,
      });
      const before = await readEvidence(expected.userServiceId);
      if (!before.state_version || before.state_version < 1) {
        throw new Error("NyxID service evidence was missing its state version.");
      }
      if (!before.api_key_id) {
        throw new Error("NyxID service evidence was missing the predecessor credential identity.");
      }
      const raw = await api.post<unknown>(
        "/assistant/actions/services/rotate-credential",
        {
          actionRequestId,
          userServiceId: expected.userServiceId,
          credential: credential.trim(),
        },
      );
      assertSecretFreeReadBack(raw);
      const response = assistantServiceRotateResponseSchema.parse(raw);
      setCredential("");
      setResultId(response.resource.userServiceId);
      const nextVersion =
        before.state_version + (response.replayed ? 0 : 1);
      const predecessorId = response.replayed ? null : before.api_key_id;
      setExpectedStateVersion(nextVersion);
      setExpectedPredecessorId(predecessorId);
      await verifyEvidence(
        response.resource.userServiceId,
        nextVersion,
        predecessorId,
        !response.replayed,
        before.updated_at,
      );
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not rotate this service credential."),
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
            {resultId
              ? "Service credential rotated"
              : "Rotate service credential"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID verified rotation through lineage ids and timestamps only. The replacement credential never left this dialog."
              : "Enter the replacement credential. NyxID stores it; the assistant never sees it."}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4">
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-rotate-credential">
                Replacement credential
              </Label>
              <Input
                id="assistant-service-rotate-credential"
                type="password"
                autoComplete="off"
                value={credential}
                onChange={(event) => setCredential(event.target.value)}
              />
            </div>
          </div>
        ) : (
          <div className="flex items-start gap-3 border-y border-border py-4">
            <Server className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 space-y-1">
              <p className="text-[13px] font-medium">Rotated service</p>
              <p className="break-all font-mono text-[12px] text-muted-foreground">
                {resultId}
              </p>
              {verified ? (
                <p className="text-[11px] text-success">
                  Rotation lineage verified. No credential material returned.
                </p>
              ) : null}
            </div>
          </div>
        )}

        {error ? (
          <p role="alert" className="text-[12px] text-destructive">
            {error}
          </p>
        ) : null}

        <DialogFooter>
          {resultId && !verified ? (
            <Button
              type="button"
              variant="outline"
              isLoading={verifying}
              onClick={() => {
                if (
                  resultId &&
                  expectedStateVersion !== null
                )
                  void verifyEvidence(
                    resultId,
                    expectedStateVersion,
                    expectedPredecessorId,
                    false,
                    undefined,
                  );
              }}
            >
              <RefreshCw />
              Retry verification
            </Button>
          ) : null}
          {resultId ? (
            <Button
              type="button"
              variant="primary"
              disabled={!verified}
              onClick={() => onComplete(resultId)}
            >
              Report service
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary"
              isLoading={submitting}
              onClick={() => void submit()}
            >
              Rotate credential
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
