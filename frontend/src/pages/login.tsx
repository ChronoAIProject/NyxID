import { useState } from "react";
import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { WebDeviceLogin } from "@/components/auth/web-device-login";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";

export function LoginPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);
  const [appLoginOpen, setAppLoginOpen] = useState(false);

  const params = new URLSearchParams(window.location.search);
  const cookieMfaRequired = params.get("mfa") === "required";
  const returnTo = params.get("return_to") ?? undefined;
  const socialError = params.get("error") ?? undefined;
  const inviteCode = params.get("code") ?? undefined;

  if (import.meta.env.DEV && import.meta.env.VITE_ROUTING_PREVIEW === "1") {
    return <div className="space-y-4"><h1 className="text-2xl font-semibold">View your real connections</h1><p className="text-sm text-muted-foreground">Sign in on NyxID, then return here to explore the local routing proposal with your production account data.</p><Button asChild><a href="/__routing-preview/login">Continue to NyxID</a></Button></div>;
  }

  // Keep QR mounted while profile restoration clears the pending MFA state,
  // so its completion effect can finish navigation.
  if (mfaRequired || cookieMfaRequired || appLoginOpen) {
    return (
      <div className="space-y-6">
        {!appLoginOpen && (
          <>
            <MfaVerifyForm returnTo={returnTo} cookieChallenge={cookieMfaRequired} />
            <p className="border-t border-border pt-6 text-center text-12 text-muted-foreground">
              You can also sign in by approving from your NyxID app.
            </p>
          </>
        )}
        <WebDeviceLogin
          returnTo={returnTo}
          isOpen={appLoginOpen}
          onOpenChange={setAppLoginOpen}
        />
      </div>
    );
  }

  return (
    <>
      {params.get("mfa") === "expired" && (
        <p role="alert" className="mb-4 rounded-lg bg-destructive/10 p-3 text-12 text-destructive">
          Your verification expired. Please sign in again.
        </p>
      )}
      <AuthFlow
        initialPanel={0}
        returnTo={returnTo}
        socialError={socialError}
        initialInviteCode={inviteCode}
      />
    </>
  );
}
