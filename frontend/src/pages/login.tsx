import { LoginForm } from "@/components/auth/login-form"
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form"
import { useAuthStore } from "@/stores/auth-store"

export function LoginPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired)

  if (mfaRequired) {
    return <MfaVerifyForm />
  }

  return <LoginForm />
}
