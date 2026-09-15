import { useParams } from "@tanstack/react-router";
import { AuthFlow } from "@/components/auth/auth-flow";
import { MfaVerifyForm } from "@/components/auth/mfa-verify-form";
import { AppConnectShell } from "@/components/connect/app-connect-shell";
import { ConnectShell } from "@/components/connect/connection-panels";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useAuthorizeContext } from "@/hooks/use-oauth-branding";
import { useAuthStore } from "@/stores/auth-store";

export function AppConnectStartPage() {
  const { ctx } = useParams({ strict: false }) as { ctx: string };
  const context = useAuthorizeContext(ctx);
  const mfaRequired = useAuthStore((state) => state.mfaRequired);
  const returnTo = new URL(
    `/oauth/authorize-context/resume?ctx=${encodeURIComponent(ctx)}`,
    window.location.origin,
  ).href;
  const socialError =
    new URLSearchParams(window.location.search).get("error") ?? undefined;
  if (context.error)
    return (
      <ConnectShell>
        <ErrorBanner message={context.error.message} />
      </ConnectShell>
    );
  if (!context.data)
    return (
      <ConnectShell>
        <p role="status">Loading sign-in request…</p>
      </ConnectShell>
    );
  const app = context.data;
  return (
    <AppConnectShell
      name={app.client_name}
      blurb={
        app.handoff_blurb ??
        `${app.client_name} uses NyxID to securely store the accounts it connects on your behalf. Sign in or create a NyxID account to continue.`
      }
      destination={app.destination}
      logoUrl={app.logo_url}
      verified={app.verified}
    >
      {mfaRequired ? (
        <MfaVerifyForm returnTo={returnTo} />
      ) : (
        <AuthFlow returnTo={returnTo} preservePath socialError={socialError} />
      )}
    </AppConnectShell>
  );
}
