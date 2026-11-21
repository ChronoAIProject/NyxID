import { ConfirmDisplayName } from "@/components/auth/confirm-display-name";
import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import { useAuthStore } from "@/stores/auth-store";

export function RegisterPage() {
  const mfaRequired = useAuthStore((s) => s.mfaRequired);
  const user = useAuthStore((s) => s.user);
  const isLoading = useAuthStore((s) => s.isLoading);
  const params = new URLSearchParams(window.location.search);
  const returnTo = params.get("return_to") ?? undefined;
  const inviteCode = params.get("code") ?? undefined;

  if (mfaRequired) {
    return <MfaVerifyForm returnTo={returnTo} />;
  }

  const confirmation = params.get("confirm_name");
  if (confirmation === "provider" || confirmation === "empty") {
    if (isLoading) return null;
    if (user?.social_provider) {
      return (
        <ConfirmDisplayName
          key={user.id}
          userId={user.id}
          initialName={
            confirmation === "provider" ? (user.display_name ?? "") : ""
          }
          onContinue={() =>
            window.location.replace(
              resolveTrustedAuthReturnTo(returnTo) ?? "/assistant",
            )
          }
        />
      );
    }
  }

  return (
    <AuthFlow
      initialPanel={1}
      returnTo={returnTo}
      initialInviteCode={inviteCode}
    />
  );
}
