import { LoginForm } from "@/components/auth/login-form";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { useAuthStore } from "@/stores/auth-store";

export function LoginPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);

  // Read return_to from the URL (set by the backend OAuth authorize flow)
  const returnTo =
    new URLSearchParams(window.location.search).get("return_to") ?? undefined;

  if (mfaRequired) {
    return <MfaVerifyForm returnTo={returnTo} />;
  }

  return <LoginForm returnTo={returnTo} />;
}
