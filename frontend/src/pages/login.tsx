import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { useAuthStore } from "@/stores/auth-store";

export function LoginPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);

  const params = new URLSearchParams(window.location.search);
  const returnTo = params.get("return_to") ?? undefined;
  const socialError = params.get("error") ?? undefined;

  if (mfaRequired) {
    return <MfaVerifyForm returnTo={returnTo} />;
  }

  return (
    <AuthFlow initialPanel={0} returnTo={returnTo} socialError={socialError} />
  );
}
