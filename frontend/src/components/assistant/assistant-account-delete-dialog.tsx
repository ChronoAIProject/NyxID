import { useRef, useState } from "react";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { ACCOUNT_DELETION_POLICY_VERSION } from "@/lib/account-deletion-policy";
import { AccountDeletionDisclosure } from "@/components/shared/account-deletion-disclosure";
import { useAuthStore } from "@/stores/auth-store";
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
  function close() { const deleted = verifiedUserId !== null; submittingRef.current = false; setConfirmEmail(""); setAccountEmail(null); setSubmitting(false); setVerifiedUserId(null); setError(null); onOpenChange(false); if (deleted) useAuthStore.getState().setUser(null); }
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
      const response = responseSchema.parse(await api.post<unknown>("/assistant/actions/org/account/delete", {
        actionRequestId,
        // Sent so the SERVER verifies the confirmation. The check above is a
        // usability guard only -- the effect route is mounted and reachable,
        // so a browser-side comparison is not a control.
        confirmEmail: confirmEmail.trim(),
        deletionPolicyVersion: ACCOUNT_DELETION_POLICY_VERSION,
      }));
      // Only a validated successful deletion response reaches this check.
      // Deletion removes the user/session, so the authenticated projection
      // normally returns 401 before its handler can return 404. Defer this
      // expected local sign-out until the completion message is dismissed;
      // the revoked credential gains no further execution authority.
      try { await apiClient("/users/me/authorization", { preserveSessionOn401: true }); throw new Error("The account still exists after deletion."); }
      catch (caught) { if (!isNotFound(caught) && !(caught instanceof ApiError && caught.status === 401)) throw caught; }
      setVerifiedUserId(response.resource.userId);
    } catch (caught) { setError(errorMessage(caught, "NyxID could not delete this account.")); }
    finally { submittingRef.current = false; setSubmitting(false); }
  }
  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader><DialogTitle>{verifiedUserId ? "Account deleted" : "Delete account"}</DialogTitle><DialogDescription>{verifiedUserId ? "NyxID confirmed account deletion. Continue to clear your session." : "Type your account email to confirm deletion of your account and the Machine data described below."}</DialogDescription></DialogHeader>
        {!verifiedUserId ? <AccountDeletionDisclosure /> : null}
        {!verifiedUserId ? <div className="space-y-2 border-y border-border py-4"><label htmlFor="assistant-account-delete-email" className="text-12 font-medium">Account email</label><Input id="assistant-account-delete-email" value={confirmEmail} onChange={(event) => setConfirmEmail(event.target.value)} placeholder={accountEmail ?? "you@example.com"} autoComplete="email" /></div> : null}
        {error ? <p role="alert" className="text-11 text-destructive">{error}</p> : null}
        <DialogFooter>{verifiedUserId ? <Button type="button" variant="primary" onClick={() => { onComplete(verifiedUserId); close(); }}>Continue</Button> : <><Button type="button" variant="outline" onClick={close}>Cancel</Button><Button type="button" variant="destructive" isLoading={submitting} disabled={submitting || !confirmEmail.trim()} onClick={() => void submit()}>Delete account</Button></>}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
