import { useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { api } from "@/lib/api-client";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";
import { assertSecretFreeReadBack, errorMessage, isNewerTimestamp } from "./assistant-action-dialog-utils";

const paramsSchema = z.object({ serviceId: actionControlIdentitySchema }).strict();
const responseSchema = z.object({ resource: z.object({ serviceId: actionControlIdentitySchema }).strict(), replayed: z.boolean() }).strict();
const evidenceSchema = z.object({ service_id: actionControlIdentitySchema, approval_required: z.boolean(), approval_mode: z.enum(["per_request", "grant"]), rule_count: z.number().int().nonnegative(), default_effect: z.enum(["require_approval", "auto_allow", "deny"]).nullable().optional(), updated_at: z.string().datetime({ offset: true }) }).passthrough();

export type AssistantApprovalConfigureParams = z.infer<typeof paramsSchema>;

export function AssistantApprovalConfigureDialog({ open, onOpenChange, actionRequestId, params, onComplete }: { readonly open: boolean; readonly onOpenChange: (open: boolean) => void; readonly actionRequestId: string; readonly params: AssistantApprovalConfigureParams; readonly onComplete: (serviceId: string) => void; }) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [approvalRequired, setApprovalRequired] = useState(true);
  const [approvalMode, setApprovalMode] = useState<"per_request" | "grant">("per_request");
  const [defaultEffect, setDefaultEffect] = useState<"require_approval" | "auto_allow" | "deny" | "">("");
  const [resultServiceId, setResultServiceId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  function close() { submittingRef.current = false; setSubmitting(false); setResultServiceId(null); setError(null); setApprovalRequired(true); setApprovalMode("per_request"); setDefaultEffect(""); onOpenChange(false); }
  async function submit() {
    if (submittingRef.current || resultServiceId) return;
    submittingRef.current = true; setSubmitting(true); setError(null);
    try {
      const expected = paramsSchema.parse(params);
      let before: z.infer<typeof evidenceSchema> | null = null;
      try { const value = await api.get<unknown>(`/approvals/service-configs/${encodeURIComponent(expected.serviceId)}/authorization`); assertSecretFreeReadBack(value); before = evidenceSchema.parse(value); } catch (caught) { if (!(caught instanceof Error && "status" in caught && (caught as { status?: number }).status === 404)) throw caught; }
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/approval/configure", { actionRequestId, serviceId: expected.serviceId, approvalRequired, approvalMode, rules: [], defaultEffect: defaultEffect || null }));
      const value = await api.get<unknown>(`/approvals/service-configs/${encodeURIComponent(expected.serviceId)}/authorization`); assertSecretFreeReadBack(value); const after = evidenceSchema.parse(value);
      if (after.service_id !== response.resource.serviceId || after.approval_required !== approvalRequired || after.approval_mode !== approvalMode || (before && !isNewerTimestamp(before.updated_at, after.updated_at))) throw new Error("NyxID could not verify the approval configuration.");
      setResultServiceId(response.resource.serviceId);
    } catch (caught) { setError(errorMessage(caught, "NyxID could not configure approvals.")); }
    finally { submittingRef.current = false; setSubmitting(false); }
  }
  return <Dialog open={open} onOpenChange={(next) => !next && close()}><DialogContent><DialogHeader><DialogTitle>{resultServiceId ? "Approval policy configured" : "Configure approvals"}</DialogTitle><DialogDescription>{resultServiceId ? "The service approval projection confirms the new policy." : "Choose the service policy NyxID should enforce."}</DialogDescription></DialogHeader>{!resultServiceId ? <div className="space-y-4 border-y border-border py-4"><div className="flex items-center justify-between gap-4"><Label htmlFor="assistant-approval-required">Require approval</Label><Switch id="assistant-approval-required" checked={approvalRequired} onCheckedChange={setApprovalRequired} /></div><label htmlFor="assistant-approval-mode" className="text-[12px] font-medium">Approval mode</label><select id="assistant-approval-mode" className="h-8 w-full rounded-lg border border-input bg-background px-3 text-[12px]" value={approvalMode} onChange={(event) => setApprovalMode(event.target.value as "per_request" | "grant")}><option value="per_request">Every request</option><option value="grant">Time-limited grant</option></select><label htmlFor="assistant-approval-effect" className="text-[12px] font-medium">Default effect</label><select id="assistant-approval-effect" className="h-8 w-full rounded-lg border border-input bg-background px-3 text-[12px]" value={defaultEffect} onChange={(event) => setDefaultEffect(event.target.value as typeof defaultEffect)}><option value="">Use required setting</option><option value="require_approval">Require approval</option><option value="auto_allow">Auto allow</option><option value="deny">Deny</option></select></div> : null}{error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}<DialogFooter>{resultServiceId ? <Button type="button" variant="primary" onClick={() => { onComplete(resultServiceId); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="primary" isLoading={submitting} disabled={submitting} onClick={() => void submit()}>Save policy</Button></>}</DialogFooter></DialogContent></Dialog>;
}
