import { useRef, useState } from "react";
import { z } from "zod";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api } from "@/lib/api-client";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";
import { assertSecretFreeReadBack, errorMessage, isNotFound } from "./assistant-action-dialog-utils";

const paramsSchema = z.object({ clientId: actionControlIdentitySchema }).strict();
const responseSchema = z.object({ resource: z.object({ userId: actionControlIdentitySchema }).strict(), replayed: z.boolean() }).strict();

export type AssistantAccountRevokeConsentParams = z.infer<typeof paramsSchema>;

export function AssistantAccountRevokeConsentDialog({
  open, onOpenChange, actionRequestId, params, onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantAccountRevokeConsentParams;
  readonly onComplete: (userId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [verifiedUserId, setVerifiedUserId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  function close() { submittingRef.current = false; setSubmitting(false); setConfirmed(false); setVerifiedUserId(null); setError(null); onOpenChange(false); }
  async function submit() {
    if (submittingRef.current || verifiedUserId) return;
    if (!confirmed) { setError("Confirm the consent revocation to continue."); return; }
    submittingRef.current = true; setSubmitting(true); setError(null);
    try {
      const expected = paramsSchema.parse(params);
      const before = await api.get<unknown>(
        `/users/me/consents/${encodeURIComponent(expected.clientId)}/authorization`,
      );
      assertSecretFreeReadBack(before);
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/account/revoke-consent", { actionRequestId, clientId: expected.clientId, confirmed }));
      try {
        const evidence = await api.get<unknown>(`/users/me/consents/${encodeURIComponent(expected.clientId)}/authorization`);
        assertSecretFreeReadBack(evidence);
        throw new Error("Consent still exists after the revoke.");
      } catch (caught) {
        if (!isNotFound(caught)) throw caught;
      }
      setVerifiedUserId(response.resource.userId);
    } catch (caught) { setError(errorMessage(caught, "NyxID could not revoke this consent.")); }
    finally { submittingRef.current = false; setSubmitting(false); }
  }
  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{verifiedUserId ? "Consent revoked" : "Revoke app consent"}</DialogTitle>
          <DialogDescription>{verifiedUserId ? "The consent projection is absent, confirming revocation." : "This app will need to ask for consent again before it can act."}</DialogDescription>
        </DialogHeader>
        {!verifiedUserId ? <div className="space-y-3 border-y border-border py-4 text-[12px]"><div className="flex items-center justify-between"><span className="text-muted-foreground">Client</span><Badge variant="secondary" className="font-mono">{params.clientId}</Badge></div><label className="flex items-start gap-2"><Checkbox checked={confirmed} onCheckedChange={(value) => setConfirmed(value === true)} /><span>I understand this app will need fresh consent.</span></label></div> : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        <DialogFooter>{verifiedUserId ? <Button type="button" variant="primary" onClick={() => { onComplete(verifiedUserId); close(); }}>Done</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="destructive" isLoading={submitting} disabled={submitting || !confirmed} onClick={() => void submit()}>Revoke consent</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
