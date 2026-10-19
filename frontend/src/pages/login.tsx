import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { useAuthStore } from "@/stores/auth-store";
import { Button } from "@/components/ui/button";

export function LoginPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);

  const params = new URLSearchParams(window.location.search);
  const returnTo = params.get("return_to") ?? undefined;
  const socialError = params.get("error") ?? undefined;
  const inviteCode = params.get("code") ?? undefined;

  if (import.meta.env.DEV && import.meta.env.VITE_ROUTING_PREVIEW === "1") {
    return <div className="space-y-4"><h1 className="text-2xl font-semibold">View your real connections</h1><p className="text-sm text-muted-foreground">Sign in on NyxID, then return here to explore the local routing proposal with your production account data.</p><Button asChild><a href="/__routing-preview/login">Continue to NyxID</a></Button></div>;
  }

  if (mfaRequired) {
    return <MfaVerifyForm returnTo={returnTo} />;
  }

  return (
    <AuthFlow
      initialPanel={0}
      returnTo={returnTo}
      socialError={socialError}
      initialInviteCode={inviteCode}
    />
  );
}
