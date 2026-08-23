import { useRef, useState } from "react";
import { KeyRound, ShieldAlert } from "lucide-react";
import { z } from "zod";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import {
  assertSecretFreeReadBack,
  assertNoSensitiveActionParams,
  errorMessage,
  isNewerTimestamp,
  SECRET_VALUE_PATTERN,
} from "./assistant-action-dialog-utils";

const serviceAccountActions = [
  "create",
  "update",
  "delete",
  "rotate_secret",
  "revoke_tokens",
] as const;
export type AssistantServiceAccountAction = (typeof serviceAccountActions)[number];

const responseSchema = z
  .object({
    resource: z.object({ serviceAccountId: z.string().min(1) }).strict(),
    replayed: z.boolean(),
    clientSecret: z.string().min(1).optional(),
  })
  .strict();

const evidenceSchema = z
  .object({
    id: z.string().min(1),
    client_id: z.string().min(1),
    role_ids: z.array(z.string()),
    is_active: z.boolean(),
    rate_limit_override: z.number().int().positive().nullable(),
    created_by: z.string().min(1),
    created_at: z.string(),
    updated_at: z.string(),
    last_authenticated_at: z.string().nullable(),
  })
  .strict();

function textParam(params: Record<string, unknown>, key: string): string {
  const value = params[key];
  return typeof value === "string" ? value : "";
}

export function AssistantServiceAccountActionDialog({
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
  readonly action: AssistantServiceAccountAction;
  readonly params: Record<string, unknown>;
  readonly onComplete: (serviceAccountId: string) => void;
}) {
  const [name, setName] = useState(textParam(params, "name"));
  const [description, setDescription] = useState(textParam(params, "description"));
  const [allowedScopes, setAllowedScopes] = useState(textParam(params, "allowedScopes") || "proxy");
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ id: string; secret?: string } | null>(null);
  const pendingRef = useRef(false);
  const destructive = action === "delete" || action === "revoke_tokens";
  const serviceAccountId = textParam(params, "serviceAccountId");

  function close() {
    pendingRef.current = false;
    setPending(false);
    setError(null);
    setConfirmed(false);
    setResult(null);
    onOpenChange(false);
  }

  async function readEvidence(id: string) {
    const raw = await api.get<unknown>(
      `/admin/service-accounts/${encodeURIComponent(id)}/authorization`,
    );
    assertSecretFreeReadBack(raw);
    return evidenceSchema.parse(raw);
  }

  async function submit() {
    if (pendingRef.current || result) return;
    if (destructive && !confirmed) {
      setError("Confirm this destructive change to continue.");
      return;
    }
    if ((action === "create" || action === "update") && SECRET_VALUE_PATTERN.test(`${name} ${description}`)) {
      setError("Names and descriptions cannot contain secret-shaped values.");
      return;
    }
    if (action === "create" && (!name.trim() || !allowedScopes.trim())) {
      setError("Enter a name and at least one allowed scope.");
      return;
    }
    if (action !== "create" && !serviceAccountId) {
      setError("The service-account reference is missing.");
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      const before = action === "create" ? null : await readEvidence(serviceAccountId);
      if (before && before.id !== serviceAccountId) {
        throw new Error("NyxID returned a different service-account identity.");
      }
      const payload: Record<string, unknown> = { ...params, actionRequestId };
      if (destructive) payload.confirmed = confirmed;
      if (action === "create" || action === "update") {
        payload.name = name.trim() || undefined;
        payload.description = description.trim() || undefined;
      }
      if (action === "create") payload.allowedScopes = allowedScopes.trim();
      if (action === "rotate_secret") payload.expectedUpdatedAt = before?.updated_at;
      const raw = await api.post<unknown>(
        `/assistant/actions/org/service-account/${action.replaceAll("_", "-")}`,
        payload,
      );
      const response = responseSchema.parse(raw);
      const id = response.resource.serviceAccountId;
      const after = await readEvidence(id);
      if (after.id !== id) throw new Error("NyxID returned a different service-account identity.");
      if (action === "create" && (!after.is_active || response.replayed === false && !response.clientSecret)) {
        throw new Error("NyxID did not return the active account and its one-time secret.");
      }
      if (action === "rotate_secret" && response.replayed === false && !response.clientSecret) {
        throw new Error("NyxID did not return the rotated secret to the browser.");
      }
      if (action === "delete" && after.is_active) {
        throw new Error("NyxID still reports this service account as active.");
      }
      if (before && !response.replayed && !isNewerTimestamp(before.updated_at, after.updated_at)) {
        throw new Error("NyxID did not show a newer service-account state.");
      }
      setResult({ id, ...(response.clientSecret ? { secret: response.clientSecret } : {}) });
    } catch (caught) {
      setError(errorMessage(caught, "NyxID could not complete this service-account action."));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }

  const title = result
    ? "Service account ready"
    : action === "rotate_secret"
      ? "Rotate service-account secret"
      : `Service account ${action.replaceAll("_", " ")}`;

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><KeyRound className="size-4" />{title}</DialogTitle>
          <DialogDescription>{result?.secret ? "Save this secret now. It will not be shown again." : result ? "The canonical service-account projection confirms the change." : destructive ? "This change revokes access and must be confirmed every time." : "NyxID keeps the service-account secret out of chat."}</DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="space-y-3 border-y border-border py-4">
            {result.secret ? <div className="space-y-2"><Label htmlFor="one-time-secret">One-time secret</Label><Input id="one-time-secret" readOnly value={result.secret} className="font-mono" /></div> : null}
            <p className="font-mono text-xs text-muted-foreground">{result.id}</p>
          </div>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            {(action === "create" || action === "update") ? <><div className="space-y-2"><Label htmlFor="sa-name">Name</Label><Input id="sa-name" value={name} onChange={(event) => setName(event.target.value)} /></div><div className="space-y-2"><Label htmlFor="sa-description">Description</Label><Input id="sa-description" value={description} onChange={(event) => setDescription(event.target.value)} /></div></> : <p className="font-mono text-xs text-muted-foreground">{serviceAccountId}</p>}
            {action === "create" ? <div className="space-y-2"><Label htmlFor="sa-scopes">Allowed scopes</Label><Input id="sa-scopes" value={allowedScopes} onChange={(event) => setAllowedScopes(event.target.value)} /></div> : null}
            {destructive ? <label className="flex items-start gap-2 text-xs"><Checkbox checked={confirmed} onCheckedChange={(value) => setConfirmed(value === true)} /><span className="flex items-center gap-1"><ShieldAlert className="size-3" />I understand this change revokes access.</span></label> : null}
          </div>
        )}
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>{result ? <Button type="button" onClick={() => { onComplete(result.id); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant={destructive ? "destructive" : "primary"} isLoading={pending} disabled={pending || (destructive && !confirmed)} onClick={() => void submit()}>{destructive ? "Confirm change" : "Continue"}</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
