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
  endpointUpdateActionParamsSchema,
} from "@/schemas/assistant-actions";

const endpointResourceSchema = z
  .object({ endpointId: actionControlIdentitySchema })
  .strict();
const assistantEndpointUpdateResponseSchema = z
  .object({
    resource: endpointResourceSchema,
    replayed: z.boolean(),
  })
  .strict();

const endpointEvidenceSchema = z
  .object({
    id: actionControlIdentitySchema,
    auto_connected: z.boolean(),
    catalog_service_id: z.string().nullable(),
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

export interface AssistantEndpointUpdateParams {
  readonly endpointId: string;
  readonly label?: string;
  readonly endpointUrl?: string;
  readonly openapiSpecUrl?: string;
}

export function AssistantEndpointUpdateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantEndpointUpdateParams;
  readonly onComplete: (endpointId: string) => void;
}) {
  const submittingRef = useRef(false);
  const verificationRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [verified, setVerified] = useState(false);
  const [result, setResult] = useState<z.infer<
    typeof assistantEndpointUpdateResponseSchema
  > | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  async function verifyUpdate(endpointId: string): Promise<void> {
    if (verificationRef.current) return;
    verificationRef.current = true;
    setVerifying(true);
    setVerified(false);
    try {
      const value = await api.get<unknown>(
        `/endpoints/${encodeURIComponent(endpointId)}/authorization`,
      );
      assertSecretFreeReadBack(value);
      const evidence = endpointEvidenceSchema.parse(value);
      if (evidence.id !== endpointId) {
        throw new Error("NyxID returned a different endpoint identity.");
      }
      if ("label" in (value as object) || "url" in (value as object)) {
        throw new Error("NyxID returned secret-bearing verification data.");
      }
      setVerified(true);
      setError(null);
    } catch (caught) {
      setError(
        errorMessage(caught, "NyxID could not verify the updated endpoint."),
      );
    } finally {
      verificationRef.current = false;
      setVerifying(false);
    }
  }

  async function updateEndpoint() {
    if (submittingRef.current || result) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const expected = endpointUpdateActionParamsSchema.parse(params);
      const response = assistantEndpointUpdateResponseSchema.parse(
        await api.post<unknown>("/assistant/actions/endpoints/update", {
          actionRequestId,
          endpointId: expected.endpointId,
          ...(expected.label !== undefined ? { label: expected.label } : {}),
          ...(expected.endpointUrl !== undefined
            ? { endpointUrl: expected.endpointUrl }
            : {}),
          ...(expected.openapiSpecUrl !== undefined
            ? { openapiSpecUrl: expected.openapiSpecUrl }
            : {}),
        }),
      );
      setResult(response);
      await verifyUpdate(response.resource.endpointId);
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not update this endpoint."));
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
            {result?.replayed ? "Endpoint already updated" : "Update endpoint"}
          </DialogTitle>
          <DialogDescription>
            {result
              ? "NyxID verified the endpoint through its authorization evidence. The assistant receives only the endpoint reference."
              : "Confirm the exact endpoint change. NyxID will not send the URL or label to the assistant."}
          </DialogDescription>
        </DialogHeader>

        {!result ? (
          <div className="space-y-3 border-y border-border py-4 text-[12px]">
            <div className="flex items-center justify-between gap-4">
              <span className="text-muted-foreground">Endpoint</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.endpointId}
              </Badge>
            </div>
            {params.label ? (
              <div className="flex items-center justify-between gap-4">
                <span className="text-muted-foreground">Label</span>
                <span className="max-w-[70%] truncate">{params.label}</span>
              </div>
            ) : null}
            {params.endpointUrl ? (
              <div className="flex items-center justify-between gap-4">
                <span className="text-muted-foreground">URL</span>
                <span className="max-w-[70%] truncate font-mono text-[11px]">
                  {params.endpointUrl}
                </span>
              </div>
            ) : null}
          </div>
        ) : null}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}

        {verified ? (
          <p className="text-[11px] text-success">
            Endpoint update verified.
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
                disabled={submitting}
                onClick={() => void updateEndpoint()}
              >
                Update endpoint
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
                    void verifyUpdate(result.resource.endpointId)
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
                  onComplete(result.resource.endpointId);
                  close();
                }}
              >
                Report endpoint
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
