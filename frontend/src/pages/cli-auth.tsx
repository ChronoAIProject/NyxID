import { useEffect, useRef } from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { api } from "@/lib/api-client";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { AlertCircle, Terminal } from "lucide-react";

interface CliAuthSearch {
  readonly port?: string;
  readonly state?: string;
}

interface TokenResponse {
  readonly access_token: string;
}

export function CliAuthPage() {
  const navigate = useNavigate();
  const { port, state } = useSearch({ strict: false }) as CliAuthSearch;
  const { isAuthenticated, isLoading } = useAuthStore();
  const callbackSent = useRef(false);

  useEffect(() => {
    if (isLoading) return;

    // Not authenticated -- redirect to login, then back here.
    // Use window.location.assign (not TanStack navigate) so that
    // return_to lands in the real URL search params where the login
    // page reads it via window.location.search.
    if (!isAuthenticated) {
      const returnPath = `/cli-auth?${new URLSearchParams({
        ...(port ? { port } : {}),
        ...(state ? { state } : {}),
      }).toString()}`;
      const returnTo = `${window.location.origin}${returnPath}`;
      window.location.assign(`/login?return_to=${encodeURIComponent(returnTo)}`);
      return;
    }

    // Authenticated and we have a CLI callback port -- send the token
    if (port && !callbackSent.current) {
      callbackSent.current = true;
      void sendTokenToCliCallback(port, state);
    }
  }, [isAuthenticated, isLoading, port, state, navigate]);

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
          <h2 className="font-display text-lg font-semibold">Invalid CLI Auth Request</h2>
          <p className="text-sm text-muted-foreground">
            This page is used by the NyxID CLI. Run{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs">nyxid login</code>{" "}
            from your terminal.
          </p>
          <Button variant="outline" onClick={() => void navigate({ to: "/" })}>
            Go to Dashboard
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center">
      <div className="flex max-w-sm flex-col items-center gap-4 text-center">
        <Terminal className="h-12 w-12 text-primary/60" />
        <h2 className="font-display text-lg font-semibold">CLI Authentication</h2>
        <p className="text-sm text-muted-foreground">
          Sending credentials to the NyxID CLI...
        </p>
        <p className="text-xs text-muted-foreground">
          You can close this tab after you see a success message in your terminal.
        </p>
      </div>
    </div>
  );
}

async function sendTokenToCliCallback(port: string, state?: string) {
  try {
    // Request a fresh access token for the CLI (uses cookie session)
    const response = await api.post<TokenResponse>("/auth/cli-token");
    const callbackUrl = new URL(`http://127.0.0.1:${port}/callback`);
    callbackUrl.searchParams.set("access_token", response.access_token);
    if (state) {
      callbackUrl.searchParams.set("state", state);
    }

    window.location.assign(callbackUrl.toString());
  } catch {
    // If refresh fails, show success page anyway -- the CLI timeout will handle it
    document.body.innerHTML = `
      <div style="display:flex;align-items:center;justify-content:center;min-height:100vh;font-family:system-ui">
        <div style="text-align:center;max-width:400px">
          <p style="font-size:14px;color:#888">Failed to send token to CLI. Please try again.</p>
        </div>
      </div>
    `;
  }
}
