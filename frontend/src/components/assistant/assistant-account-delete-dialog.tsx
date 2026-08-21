import { useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api-client";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";
import { assertSecretFreeReadBack, errorMessage, isNotFound } from "./assistant-action-dialog-utils";

const paramsSchema = z.object({}).strict();
const responseSchema = z.object({ resource: z.object({ userId: actionControlIdentitySchema }).strict(), replayed: z.boolean() }).strict();

export function AssistantAccountDeleteDialog({
  open, onOpenChange, actionRequestId, onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly onComplete: (userId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [confirmEmail, setConfirmEmail] = useState("");
  const [accountEmail, setAccountEmail] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [verifiedUserId, setVerifiedUserId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  function close() { submittingRef.current = false; setConfirmEmail(""); setAccountEmail(null); setSubmitting(false); setVerifiedUserId(null); setError(null); onOpenChange(false); }
  async function submit() {
    if (submittingRef.current || verifiedUserId) return;
    submittingRef.current = true; setSubmitting(true); setError(null);
    try {
      paramsSchema.parse({});
      const profile = await api.get<{ email: string }>("/users/me");
      setAccountEmail(profile.email);
      if (confirmEmail.trim().toLowerCase() !== profile.email.trim().toLowerCase()) throw new Error("Type your account email exactly to continue.");
      const before = await api.get<unknown>("/users/me/authorization");
      assertSecretFreeReadBack(before);
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/account/delete", { actionRequestId }));
      try { await api.get(`/users/me/authorization`); throw new Error("The account still exists after deletion."); }
      catch (caught) { if (!isNotFound(caught)) throw caught; }
      setVerifiedUserId(response.resource.userId);
    } catch (caught) { setError(errorMessage(caught, "NyxID could not delete this account.")); }
    finally { submittingRef.current = false; setSubmitting(false); }
  }
  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader><DialogTitle>{verifiedUserId ? "Account deleted" : "Delete account"}</DialogTitle><DialogDescription>{verifiedUserId ? "The account projection is absent. Your session can now be cleared." : "This is irreversible. Every time, type your account email to confirm."}</DialogDescription></DialogHeader>
        {!verifiedUserId ? <div className="space-y-2 border-y border-border py-4"><label htmlFor="assistant-account-delete-email" className="text-[12px] font-medium">Account email</label><Input id="assistant-account-delete-email" value={confirmEmail} onChange={(event) => setConfirmEmail(event.target.value)} placeholder={accountEmail ?? "you@example.com"} autoComplete="email" /></div> : null}
        {error ? <p role="alert" className="text-[11px] text-destructive">{error}</p> : null}
        <DialogFooter>{verifiedUserId ? <Button type="button" variant="primary" onClick={() => { onComplete(verifiedUserId); close(); }}>Continue</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="destructive" isLoading={submitting} disabled={submitting || !confirmEmail.trim()} onClick={() => void submit()}>Delete account</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
