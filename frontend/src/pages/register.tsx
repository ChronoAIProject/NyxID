import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { useAuthStore } from "@/stores/auth-store";

export function RegisterPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);

  const params = new URLSearchParams(window.location.search);
  const returnTo = params.get("return_to") ?? undefined;

  if (mfaRequired) {
    return <MfaVerifyForm returnTo={returnTo} />;
  }

  return <AuthFlow initialPanel={1} returnTo={returnTo} />;
}
