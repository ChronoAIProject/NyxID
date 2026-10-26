import { useState } from "react";
import { useAuthStore } from "@/stores/auth-store";
import { useSaveSecurityPreferences } from "@/hooks/use-security-preferences";
import { useMfaStepUp } from "@/hooks/use-mfa-step-up";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";

export function ApprovedSignInPolicy() {
  const user = useAuthStore((s) => s.user);
  const accountId = user?.id;
  const policy = user?.profile_config?.security;
  const save = useSaveSecurityPreferences();
  const [confirmAccount, setConfirmAccount] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stepUp = useMfaStepUp(`security-approved-sign-ins:${accountId}`);

  async function persist(enabled: boolean) {
    if (!accountId || useAuthStore.getState().user?.id !== accountId) return;
    setError(null);
    try {
      await save.mutateAsync({ accountId, skip_mfa_for_approved_sign_ins: enabled });
    } catch (failure) {
      if (useAuthStore.getState().user?.id !== accountId) return;
      if (!stepUp.handle(failure, () => persist(enabled))) {
        setError(failure instanceof Error ? failure.message : "Could not save this setting. Try again.");
      }
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <label htmlFor="skip-approved-sign-in-mfa" className="text-12 font-medium">
            Skip MFA for NyxID app sign-ins
          </label>
          <p id="skip-approved-sign-in-description" className="text-12 text-muted-foreground">
            Approve QR or typed sign-in codes from a signed-in NyxID app or browser without
            another authenticator code. Enabled by default; turn this off to require MFA
            for these approvals. Approve only codes shown on your own screen.
            {!user?.mfa_enabled && " Takes effect when two-factor authentication is on."}
          </p>
          {!policy && <p className="text-12 text-muted-foreground">This server does not support changing this setting yet.</p>}
        </div>
        <Switch
          id="skip-approved-sign-in-mfa"
          aria-describedby="skip-approved-sign-in-description"
          checked={policy?.skip_mfa_for_approved_sign_ins ?? false}
          disabled={!policy || save.isPending || !!stepUp.prompt}
          onCheckedChange={(enabled) => {
            setError(null);
            if (enabled) setConfirmAccount(accountId ?? null);
            else void persist(false);
          }}
        />
      </div>
      {error && <p role="alert" className="text-12 text-destructive">{error}</p>}
      {stepUp.prompt}
      <Dialog open={!!accountId && confirmAccount === accountId} onOpenChange={(open) => { if (!open) setConfirmAccount(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Skip MFA for app-approved sign-ins?</DialogTitle>
            <DialogDescription>
              Anyone with access to your signed-in NyxID app or browser can approve another
              sign-in without an authenticator code. Review each request and approve
              only codes shown on your own screen. You can turn this off at any time.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setConfirmAccount(null)}>Cancel</Button>
            <Button variant="primary" onClick={() => { setConfirmAccount(null); void persist(true); }}>Enable for app sign-ins</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
