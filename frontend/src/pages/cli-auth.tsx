import { useMfaStepUp } from "@/hooks/use-mfa-step-up";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { api } from "@/lib/api-client";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { AlertCircle } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { buildCliAuthReturnPath, type CliAuthSearch } from "./cli-auth.helpers";

interface TokenResponse {
  readonly access_token: string;
  readonly refresh_token: string;
}

export function CliAuthPage() {
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
  const callbackSent = useRef(false);

  useEffect(() => {
    if (isLoading) return;

    // Not authenticated -- redirect to login, then back here.
    // Use window.location.assign (not TanStack navigate) so that
    // return_to lands in the real URL search params where the login
    // page reads it via window.location.search.
    if (!isAuthenticated) {
      const returnPath = buildCliAuthReturnPath({ port, state, client_ua });
      const returnTo = `${window.location.origin}${returnPath}`;
      window.location.assign(
        `/login?return_to=${encodeURIComponent(returnTo)}`,
      );
      return;
    }

    // Authenticated and we have a CLI callback port -- send the token
    if (port && !callbackSent.current) {
      callbackSent.current = true;
      const send = async () => {
        try {
          await sendTokenToCliCallback(port, state, client_ua);
        } catch (failure) {
          if (handleStepUp(failure, send)) return;
          setError(
            `Failed to send token to CLI. ${failure instanceof Error ? failure.message : "Please try again."}`,
          );
        }
      };
      void send();
    }
  }, [
    isAuthenticated,
    isLoading,
    port,
    state,
    client_ua,
    navigate,
    handleStepUp,
  ]);

  if (isLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <Skeleton className="h-32 w-80" />
      </div>
    );
  }

  if (!port) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="flex max-w-sm flex-col items-center gap-4 text-center">
          <AlertCircle className="h-12 w-12 text-muted-foreground/50" />
          <h2 className="text-lg font-semibold">
            Invalid CLI Auth Request
          </h2>
          <p className="text-12 text-muted-foreground">
            This page is used by the NyxID CLI. Run{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">
              nyxid login
            </code>{" "}
            from your terminal.
          </p>
          <Button variant="outline" onClick={() => void navigate({ to: "/dashboard" })}>
            Go to Dashboard
          </Button>
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
        <h2 className="text-lg font-semibold">
          CLI Authentication
        </h2>
        {!error && (
          <>
            <p className="text-12 text-muted-foreground">
              Sending credentials to the NyxID CLI...
            </p>
            <p className="text-xs text-muted-foreground">
              You can close this tab after you see a success message in your
              terminal.
            </p>
          </>
        )}
      </div>
    </div>
  );
}

async function sendTokenToCliCallback(
  port: string,
  state?: string,
  clientUa?: string,
) {
  // Request a fresh access token for the CLI (uses cookie session).
  // Pass through the CLI's user-agent so the session is identifiable.
  const response = await api.post<TokenResponse>("/auth/cli-token", {
    client_ua: clientUa,
  });
  const callbackUrl = new URL(`http://127.0.0.1:${port}/callback`);
  callbackUrl.searchParams.set("access_token", response.access_token);
  callbackUrl.searchParams.set("refresh_token", response.refresh_token);
  if (state) {
    callbackUrl.searchParams.set("state", state);
  }

  window.location.assign(callbackUrl.toString());
}
