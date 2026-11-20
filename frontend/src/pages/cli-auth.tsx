import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useMfaStepUp } from "@/hooks/use-mfa-step-up";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { api } from "@/lib/api-client";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { AlertCircle } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { buildCliAuthReturnPath, cliCallbackUrl, type CliAuthSearch } from "./cli-auth.helpers";

interface TokenResponse {
  readonly access_token: string;
  readonly refresh_token: string;
}

export function CliAuthPage() {
  useUiTranslation();
  const navigate = useNavigate();
  const { port, state, client_ua } = useSearch({
    strict: false,
  }) as CliAuthSearch;
  const { user, isAuthenticated, isLoading } = useAuthStore();
  const [error, setError] = useState<string | null>(null);
  const { handle: handleStepUp, prompt: stepUpPrompt } = useMfaStepUp(
    `${user?.id ?? ""}:${port}:${state}`,
    () => setError("Verification cancelled. Run nyxid login again."),
  );
  const callback = cliCallbackUrl(port)?.href;
  const [sending, setSending] = useState(false);
  const callbackSent = useRef(false);

  useEffect(() => {
    if (isLoading || isAuthenticated || !callback) return;

    // Not authenticated -- redirect to login, then back here.
    // Use window.location.assign (not TanStack navigate) so that
    // return_to lands in the real URL search params where the login
    // page reads it via window.location.search.
    const returnPath = buildCliAuthReturnPath({ port, state, client_ua });
    const returnTo = `${window.location.origin}${returnPath}`;
    window.location.assign(`/login?return_to=${encodeURIComponent(returnTo)}`);
  }, [isAuthenticated, isLoading, callback, port, state, client_ua]);

  async function send() {
    if (!callback || !isAuthenticated) return;
    try {
      await sendTokenToCliCallback(new URL(callback), state, client_ua);
    } catch (failure) {
      if (handleStepUp(failure, send)) return;
      setError(
        `Failed to send token to CLI. ${failure instanceof Error ? failure.message : "Please try again."}`,
      );
    }
  }

  function confirm() {
    if (callbackSent.current) return;
    callbackSent.current = true;
    setSending(true);
    void send();
  }

  if (isLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <Skeleton className="h-32 w-80" />
      </div>
    );
  }

  if (!callback) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="flex max-w-sm flex-col items-center gap-4 text-center">
          <AlertCircle className="h-12 w-12 text-muted-foreground/50" />
          <h2 className="text-lg font-semibold">{translateUi("Invalid CLI Auth Request")}</h2>
          <p className="text-12 text-muted-foreground">{translateUi("This page is used by the NyxID CLI. Run")}{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">
              nyxid login
            </code>{" "}{translateUi("from your terminal.")}</p>
          <Button variant="outline" onClick={() => void navigate({ to: "/dashboard" })}>{translateUi("Go to Dashboard")}</Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center">
      <div className="flex max-w-sm flex-col items-center gap-4 text-center">
        <NyxidIcon className="h-12 w-12" />
        {stepUpPrompt}
        {error && (
          <p role="alert" className="text-12 text-destructive">
            {error}
          </p>
        )}
        <h2 className="text-lg font-semibold">{translateUi("CLI Authentication")}</h2>
        {!sending && isAuthenticated && (
          <>
            <p className="text-12 text-muted-foreground">{translateUi("Authorize full account access for the CLI you started on this computer?")}</p>
            <code className="text-xs">{callback}</code>
            <Button onClick={confirm}>{translateUi("Authorize CLI sign-in")}</Button>
          </>
        )}
        {sending && !error && !stepUpPrompt && (
          <>
            <p className="text-12 text-muted-foreground">{translateUi("Sending credentials to the NyxID CLI...")}</p>
            <p className="text-xs text-muted-foreground">{translateUi("You can close this tab after you see a success message in your terminal.")}</p>
          </>
        )}
      </div>
    </div>
  );
}

async function sendTokenToCliCallback(
  callbackUrl: URL,
  state?: string,
  clientUa?: string,
) {
  // Request a fresh access token for the CLI (uses cookie session).
  // Pass through the CLI's user-agent so the session is identifiable.
  const response = await api.post<TokenResponse>("/auth/cli-token", {
    client_ua: clientUa,
  });
  callbackUrl.searchParams.set("access_token", response.access_token);
  callbackUrl.searchParams.set("refresh_token", response.refresh_token);
  if (state) {
    callbackUrl.searchParams.set("state", state);
  }

  window.location.assign(callbackUrl.toString());
}
