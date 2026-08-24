import { useRef, useState } from "react";
import { Cloud, PlugZap } from "lucide-react";
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
import { api } from "@/lib/api-client";
import { assistantOneTimeMaterialSchema } from "@/schemas/assistant-action-effects";
import {
  assertNoSensitiveActionParams,
  assertSecretFreeReadBack,
  errorMessage,
} from "./assistant-action-dialog-utils";

export type AssistantOrgIntegrationAction =
  | "external_key.add_gcp_service_account"
  | "openclaw.connect";

const externalResponseSchema = z
  .object({
    resource: z.object({ externalKeyId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    oneTimeMaterial: assistantOneTimeMaterialSchema,
  })
  .strict();
const externalEvidenceSchema = z
  .object({
    id: z.string().min(1),
    credential_type: z.enum(["api_key", "oauth2", "bearer", "basic", "ssh_certificate", "node_managed", "gcp_service_account"]),
    status: z.enum(["active", "expired", "revoked", "failed", "refresh_failed", "pending_auth"]),
    expires_at: z.string().nullable(),
    last_used_at: z.string().nullable(),
    updated_at: z.string(),
  })
  .strict();

const serviceResponseSchema = z
  .object({
    resource: z.object({ userServiceId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    oneTimeMaterial: assistantOneTimeMaterialSchema,
  })
  .strict();
const serviceEvidenceSchema = z
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

function textParam(params: Record<string, unknown>, key: string): string {
  const value = params[key];
  return typeof value === "string" ? value : "";
}

export function AssistantOrgIntegrationActionDialog({
  open,
  onOpenChange,
  actionRequestId,
  action,
  params,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly action: AssistantOrgIntegrationAction;
  readonly params: Record<string, unknown>;
  readonly onComplete: (resourceId: string) => void;
}) {
  const [label, setLabel] = useState(textParam(params, "label"));
  const [keyJson, setKeyJson] = useState("");
  const [scopes, setScopes] = useState("https://www.googleapis.com/auth/cloud-platform");
  const [serviceSlugs, setServiceSlugs] = useState("");
  const [gatewayUrl, setGatewayUrl] = useState(textParam(params, "gatewayUrl"));
  const [credential, setCredential] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const gcp = action === "external_key.add_gcp_service_account";
  const targetOrgId = textParam(params, "targetOrgId");

  function close() {
    pendingRef.current = false;
    setPending(false);
    setError(null);
    setKeyJson("");
    setCredential("");
    setResultId(null);
    onOpenChange(false);
  }

  async function submit() {
    if (pendingRef.current || resultId) return;
    if (gcp && !keyJson.trim()) {
      setError("Paste the GCP service-account JSON in this browser dialog.");
      return;
    }
    if (!gcp && (!gatewayUrl.trim() || !credential.trim())) {
      setError("Enter the OpenClaw gateway URL and bearer credential.");
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      if (gcp) {
        const response = externalResponseSchema.parse(
          await api.post<unknown>("/assistant/actions/org/external-key/add-gcp-service-account", {
            actionRequestId,
            label: label.trim() || undefined,
            keyJson,
            scopes: scopes.trim() || undefined,
            serviceSlugs: serviceSlugs.split(",").map((slug) => slug.trim()).filter(Boolean),
            targetOrgId: targetOrgId || undefined,
          }),
        );
        const raw = await api.get<unknown>(
          `/api-keys/external/${encodeURIComponent(response.resource.externalKeyId)}/authorization`,
        );
        assertSecretFreeReadBack(raw);
        const evidence = externalEvidenceSchema.parse(raw);
        if (evidence.id !== response.resource.externalKeyId || evidence.credential_type !== "gcp_service_account" || evidence.status !== "active") {
          throw new Error("NyxID did not show the active GCP external credential.");
        }
        setKeyJson("");
        setResultId(evidence.id);
      } else {
        const response = serviceResponseSchema.parse(
          await api.post<unknown>("/assistant/actions/org/openclaw/connect", {
            actionRequestId,
            gatewayUrl: gatewayUrl.trim(),
            credential,
            label: label.trim() || undefined,
          }),
        );
        const raw = await api.get<unknown>(
          `/keys/${encodeURIComponent(response.resource.userServiceId)}/authorization`,
        );
        assertSecretFreeReadBack(raw);
        const evidence = serviceEvidenceSchema.parse(raw);
        if (evidence.id !== response.resource.userServiceId || !evidence.is_active || evidence.status !== "active") {
          throw new Error("NyxID did not show the active OpenClaw service.");
        }
        setCredential("");
        setResultId(evidence.id);
      }
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not complete this integration action."));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">{gcp ? <Cloud className="size-4" /> : <PlugZap className="size-4" />}{resultId ? "Connection confirmed" : gcp ? "Add GCP service account" : "Connect OpenClaw"}</DialogTitle>
          <DialogDescription>{resultId ? "The canonical typed projection confirms the new resource." : "Sensitive material is submitted directly from this browser dialog and is never returned to chat."}</DialogDescription>
        </DialogHeader>
        {!resultId ? (
          <div className="space-y-4 border-y border-border py-4">
            <div className="space-y-2"><Label htmlFor="integration-label">Label</Label><Input id="integration-label" value={label} onChange={(event) => setLabel(event.target.value)} autoComplete="off" /></div>
            {gcp ? <><div className="space-y-2"><Label htmlFor="gcp-key-json">Service-account JSON</Label><textarea id="gcp-key-json" value={keyJson} onChange={(event) => setKeyJson(event.target.value)} className="min-h-32 w-full border border-input bg-background px-3 py-2 font-mono text-xs" autoComplete="off" /></div><div className="space-y-2"><Label htmlFor="gcp-scopes">OAuth scopes</Label><Input id="gcp-scopes" value={scopes} onChange={(event) => setScopes(event.target.value)} /></div><div className="space-y-2"><Label htmlFor="gcp-services">Service slugs</Label><Input id="gcp-services" value={serviceSlugs} onChange={(event) => setServiceSlugs(event.target.value)} /></div>{targetOrgId ? <div className="space-y-1.5"><p className="text-xs font-medium">Target organization</p><p className="break-all font-mono text-xs text-muted-foreground">{targetOrgId}</p></div> : null}</> : <><div className="space-y-2"><Label htmlFor="openclaw-url">Gateway URL</Label><Input id="openclaw-url" type="url" value={gatewayUrl} onChange={(event) => setGatewayUrl(event.target.value)} autoComplete="url" /></div><div className="space-y-2"><Label htmlFor="openclaw-credential">Bearer credential</Label><Input id="openclaw-credential" type="password" value={credential} onChange={(event) => setCredential(event.target.value)} autoComplete="new-password" /></div></>}
          </div>
        ) : <p className="border-y border-border py-4 font-mono text-xs text-muted-foreground">{resultId}</p>}
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>{resultId ? <Button type="button" onClick={() => { onComplete(resultId); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="primary" isLoading={pending} disabled={pending} onClick={() => void submit()}>Continue</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
