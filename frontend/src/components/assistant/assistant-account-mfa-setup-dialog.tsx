import { useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api-client";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";
import { assistantOneTimeMaterialSchema } from "@/schemas/assistant-action-effects";
import { assertSecretFreeReadBack, errorMessage } from "./assistant-action-dialog-utils";

const responseSchema = z.object({
  resource: z.object({ userId: actionControlIdentitySchema }).strict(),
  stage: z.enum(["start", "confirm"]),
  factorId: actionControlIdentitySchema.nullable().optional(),
  setupValue: z.string().min(1).nullable().optional(),
  qrCodeUrl: z.string().url().nullable().optional(),
  recoveryValues: z.array(z.string().min(1)).nullable().optional(),
  replayed: z.boolean(),
  oneTimeMaterial: assistantOneTimeMaterialSchema,
}).strict();

const evidenceSchema = z.object({
  id: actionControlIdentitySchema,
  mfa_enabled: z.boolean(),
}).passthrough();

export function AssistantAccountMfaSetupDialog({
  open, onOpenChange, actionRequestId, onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly onComplete: (userId: string) => void;
}) {
  const startRef = useRef(false);
  const confirmRef = useRef(false);
  const [stage, setStage] = useState<"start" | "confirm" | "complete">("start");
  const [factorId, setFactorId] = useState<string | null>(null);
  const [setupValue, setSetupValue] = useState<string | null>(null);
  const [recoveryValues, setRecoveryValues] = useState<string[] | null>(null);
  const [code, setCode] = useState("");
  const [userId, setUserId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function close() {
    startRef.current = false; confirmRef.current = false; setStage("start"); setFactorId(null); setSetupValue(null); setRecoveryValues(null); setCode(""); setUserId(null); setSubmitting(false); setError(null); onOpenChange(false);
  }

  async function start() {
    if (startRef.current || stage !== "start") return;
    startRef.current = true; setSubmitting(true); setError(null);
    try {
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/account/mfa-setup", { actionRequestId, stage: "start" }));
      if (!response.factorId || !response.setupValue) throw new Error("NyxID did not return the one-time MFA setup material.");
      setFactorId(response.factorId); setSetupValue(response.setupValue); setStage("confirm");
    } catch (caught) { setError(errorMessage(caught, "NyxID could not start MFA setup.")); }
    finally { startRef.current = false; setSubmitting(false); }
  }

  async function confirm() {
    if (confirmRef.current || stage !== "confirm" || !factorId || code.length !== 6) return;
    confirmRef.current = true; setSubmitting(true); setError(null);
    try {
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/account/mfa-setup", { actionRequestId, stage: "confirm", factorId, code }));
      const evidenceValue = await api.get<unknown>("/users/me/authorization");
      assertSecretFreeReadBack(evidenceValue);
      const evidence = evidenceSchema.parse(evidenceValue);
      if (evidence.id !== response.resource.userId || !evidence.mfa_enabled) throw new Error("NyxID could not verify MFA enrollment.");
      setUserId(response.resource.userId); setRecoveryValues(response.recoveryValues ?? null); setStage("complete");
    } catch (caught) { setError(errorMessage(caught, "NyxID could not confirm MFA setup.")); }
    finally { confirmRef.current = false; setSubmitting(false); }
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader><DialogTitle>{stage === "start" ? "Set up MFA" : stage === "confirm" ? "Verify MFA setup" : "MFA enabled"}</DialogTitle><DialogDescription>{stage === "start" ? "NyxID will show the authenticator secret once in this browser." : stage === "confirm" ? "Enter the six-digit code from your authenticator. Recovery codes are shown once." : "The account projection confirms MFA is enabled."}</DialogDescription></DialogHeader>
        {stage === "start" ? <p className="text-[12px] text-muted-foreground">No setup request is sent until you click Start setup.</p> : null}
        {stage === "confirm" && setupValue ? <div className="space-y-3 border-y border-border py-4"><div><p className="mb-1 text-[11px] text-muted-foreground">One-time authenticator secret</p><code className="block break-all rounded-lg border border-border bg-muted px-3 py-2 font-mono text-xs">{setupValue}</code></div><label htmlFor="assistant-mfa-code" className="text-[12px] font-medium">Authenticator code</label><Input id="assistant-mfa-code" inputMode="numeric" maxLength={6} value={code} onChange={(event) => setCode(event.target.value.replace(/\D/g, ""))} /></div> : null}
        {stage === "complete" && recoveryValues ? <div className="space-y-2 border-y border-border py-4"><p className="text-[11px] text-muted-foreground">Save these recovery codes now. They are not sent to the assistant.</p><div className="grid grid-cols-2 gap-2">{recoveryValues.map((value) => <code key={value} className="rounded-lg border border-border bg-muted px-2 py-1 text-center font-mono text-xs">{value}</code>)}</div></div> : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        <DialogFooter>{stage === "start" ? <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="primary" isLoading={submitting} disabled={submitting} onClick={() => void start()}>Start setup</Button></> : stage === "confirm" ? <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="primary" isLoading={submitting} disabled={submitting || code.length !== 6} onClick={() => void confirm()}>Enable MFA</Button></> : <Button type="button" variant="primary" onClick={() => { if (userId) onComplete(userId); close(); }}>I have saved them</Button>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
