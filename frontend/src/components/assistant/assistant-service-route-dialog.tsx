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
  serviceRouteActionParamsSchema,
} from "@/schemas/assistant-actions";

const serviceResourceSchema = z
  .object({ userServiceId: actionControlIdentitySchema })
  .strict();
const assistantServiceRouteResponseSchema = z
  .object({
    resource: serviceResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

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

export interface AssistantServiceRouteParams {
  readonly userServiceId: string;
  readonly viaNodeId?: string;
}

export function AssistantServiceRouteDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantServiceRouteParams;
  readonly onComplete: (userServiceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [viaNodeId, setViaNodeId] = useState(params.viaNodeId ?? "");
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);
  const [observedNodeId, setObservedNodeId] = useState<string | null>(null);

  function close() {
    setError(null);
    setVerified(false);
    setResultId(null);
    setObservedNodeId(null);
    submittingRef.current = false;
    verificationRef.current = false;
    setSubmitting(false);
    setVerifying(false);
    onOpenChange(false);
  }

  async function verifyEvidence(
    userServiceId: string,
    expectedNodeId: string | undefined,
  ): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const value = await api.get<unknown>(
        `/keys/${encodeURIComponent(userServiceId)}/authorization`,
      );
      assertSecretFreeReadBack(value);
      const evidence = authorizationEvidenceSchema.parse(value);
      if (evidence.id !== userServiceId) {
        throw new Error("NyxID returned a different service identity.");
      }
      const expected = expectedNodeId?.trim() ? expectedNodeId.trim() : null;
      if (evidence.node_id !== expected) {
        throw new Error("NyxID routing evidence did not match this action.");
      }
      setObservedNodeId(evidence.node_id);
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify this routing change."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function submit() {
    if (submittingRef.current || resultId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = serviceRouteActionParamsSchema.parse({
        userServiceId: params.userServiceId,
        ...(viaNodeId.trim() ? { viaNodeId: viaNodeId.trim() } : {}),
      });
      const response = assistantServiceRouteResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/services/route", {
          actionRequestId,
          userServiceId: expected.userServiceId,
          ...(expected.viaNodeId ? { viaNodeId: expected.viaNodeId } : {}),
        }),
      );
      setResultId(response.resource.userServiceId);
      await verifyEvidence(response.resource.userServiceId, expected.viaNodeId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not route this service."));
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
            {resultId ? "Service routing updated" : "Route connected service"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID verified node routing from authorization evidence."
              : "Set a node UUID to route through a credential node, or leave empty to clear routing."}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4">
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-route-node">Node ID</Label>
              <Input
                id="assistant-service-route-node"
                value={viaNodeId}
                onChange={(event) => setViaNodeId(event.target.value)}
                placeholder="Leave empty to clear routing"
                maxLength={256}
              />
            </div>
          </div>
        ) : (
          <div className="flex items-start gap-3 border-y border-border py-4">
            <Server className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 space-y-1">
              <p className="text-[13px] font-medium">
                {observedNodeId ? "Routed through node" : "Direct routing"}
              </p>
              <p className="break-all font-mono text-[12px] text-muted-foreground">
                {observedNodeId ?? "no node"}
              </p>
              {verified ? (
                <p className="text-[11px] text-success">
                  Routing evidence verified.
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
                if (resultId) void verifyEvidence(resultId, viaNodeId);
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
              Apply routing
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
