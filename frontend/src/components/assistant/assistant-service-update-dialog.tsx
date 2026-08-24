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
  customServiceAuthMethodSchema,
  serviceUpdateActionParamsSchema,
} from "@/schemas/assistant-actions";

const serviceResourceSchema = z
  .object({ userServiceId: actionControlIdentitySchema })
  .strict();
const assistantServiceUpdateResponseSchema = z
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

export interface AssistantServiceUpdateParams {
  readonly userServiceId: string;
  readonly name?: string;
  readonly endpointUrl?: string;
  readonly authMethod?: z.infer<typeof customServiceAuthMethodSchema>;
  readonly authKeyName?: string;
}

export function AssistantServiceUpdateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantServiceUpdateParams;
  readonly onComplete: (userServiceId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [replayed, setReplayed] = useState(false);
  const [name, setName] = useState(params.name ?? "");
  const [endpointUrl, setEndpointUrl] = useState(params.endpointUrl ?? "");
  const [authMethod, setAuthMethod] = useState(params.authMethod ?? "");
  const [authKeyName, setAuthKeyName] = useState(params.authKeyName ?? "");
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);
  const [expectedStateVersion, setExpectedStateVersion] = useState<number | null>(null);

  function close() {
    setError(null);
    setVerified(false);
    setReplayed(false);
    setResultId(null);
    setExpectedStateVersion(null);
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
    expectedStateVersion: number,
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
      if (evidence.state_version !== expectedStateVersion) {
        throw new Error("NyxID service evidence did not show the expected state advance.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify this service update."),
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
      const expected = serviceUpdateActionParamsSchema.parse({
        userServiceId: params.userServiceId,
        ...(name.trim() ? { name: name.trim() } : {}),
        ...(endpointUrl.trim() ? { endpointUrl: endpointUrl.trim() } : {}),
        ...(authMethod.trim() ? { authMethod: authMethod.trim() } : {}),
        ...(authKeyName.trim() ? { authKeyName: authKeyName.trim() } : {}),
      });
      const before = await readEvidence(expected.userServiceId);
      if (!before.state_version || before.state_version < 1) {
        throw new Error("NyxID service evidence was missing its state version.");
      }
      const response = assistantServiceUpdateResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/services/update", {
          actionRequestId,
          userServiceId: expected.userServiceId,
          ...(expected.name ? { name: expected.name } : {}),
          ...(expected.endpointUrl
            ? { endpointUrl: expected.endpointUrl }
            : {}),
          ...(expected.authMethod ? { authMethod: expected.authMethod } : {}),
          ...(expected.authKeyName
            ? { authKeyName: expected.authKeyName }
            : {}),
        }),
      );
      setResultId(response.resource.userServiceId);
      setReplayed(response.replayed);
      const expectedVersion =
        before.state_version + (response.replayed ? 0 : 1);
      setExpectedStateVersion(expectedVersion);
      await verifyEvidence(
        response.resource.userServiceId,
        expectedVersion,
      );
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not update this service."));
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
              ? replayed
                ? "Service already updated"
                : "Service updated"
              : "Update connected service"}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "NyxID verified the change from authorization evidence. The assistant receives only the service reference."
              : "Confirm the fields NyxID will change. Your credential stays in NyxID."}
          </DialogDescription>
        </DialogHeader>

        {!resultId ? (
          <div className="space-y-3 border-y border-border py-4">
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-update-name">Name</Label>
              <Input
                id="assistant-service-update-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                maxLength={4_096}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-update-url">Endpoint URL</Label>
              <Input
                id="assistant-service-update-url"
                value={endpointUrl}
                onChange={(event) => setEndpointUrl(event.target.value)}
                maxLength={4_096}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-update-auth">Auth method</Label>
              <Input
                id="assistant-service-update-auth"
                value={authMethod}
                onChange={(event) => setAuthMethod(event.target.value)}
                maxLength={64}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="assistant-service-update-key-name">
                Auth key name
              </Label>
              <Input
                id="assistant-service-update-key-name"
                value={authKeyName}
                onChange={(event) => setAuthKeyName(event.target.value)}
                maxLength={256}
              />
            </div>
          </div>
        ) : (
          <div className="flex items-start gap-3 border-y border-border py-4">
            <Server className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 space-y-1">
              <p className="text-[13px] font-medium">Connected service</p>
              <p className="break-all font-mono text-[12px] text-muted-foreground">
                {resultId}
              </p>
              {verified ? (
                <p className="text-[11px] text-success">
                  Authorization evidence verified.
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
                if (resultId && expectedStateVersion !== null)
                  void verifyEvidence(resultId, expectedStateVersion);
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
              Update service
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
