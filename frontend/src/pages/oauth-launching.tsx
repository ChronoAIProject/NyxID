import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { OAUTH_PROVIDER_ORIGIN_KEY } from "@/lib/oauth-popup";
import { validateAuthorizationUrl } from "@/schemas/oauth-popup";
import type { OAuthLaunchNavigateMessage } from "@/types/oauth-popup";

function launchIdFromWindowName(name: string): string | null {
  const value = name.startsWith("nyxid_oauth_")
    ? name.slice("nyxid_oauth_".length)
    : "";
  return /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
    value,
  )
    ? value
    : null;
}

export function OAuthLaunchingPage() {
  const [launchContext] = useState(() => ({
    launchId: launchIdFromWindowName(window.name),
    opener: window.opener,
  }));
  const failed = !launchContext.launchId || !launchContext.opener;

  useEffect(() => {
    const { launchId, opener } = launchContext;
    if (!launchId || !opener) {
      window.opener = null;
      return;
    }

    const onMessage = (event: MessageEvent<unknown>) => {
      const message = event.data as Partial<OAuthLaunchNavigateMessage> | null;
      if (
        event.origin !== window.location.origin ||
        event.source !== opener ||
        message?.type !== "oauth_launch_navigate" ||
        message.launchId !== launchId ||
        typeof message.url !== "string" ||
        typeof message.nonce !== "string" ||
        !validateAuthorizationUrl(message.url, message.nonce)
      ) {
        return;
      }
      window.removeEventListener("message", onMessage);
      sessionStorage.setItem(
        OAUTH_PROVIDER_ORIGIN_KEY,
        new URL(message.url, window.location.origin).origin,
      );
      window.location.assign(message.url);
    };
    window.addEventListener("message", onMessage);

    // Navigation is impossible until this page confirms opener severance.
    window.opener = null;
    opener.postMessage(
      { type: "oauth_launch_ready", launchId },
      window.location.origin,
    );

    return () => window.removeEventListener("message", onMessage);
  }, [launchContext]);

  return (
    <main className="flex min-h-screen items-center justify-center bg-background px-6 text-foreground">
      <section className="w-full max-w-sm rounded-lg border border-border bg-card p-6 text-center shadow-lg">
        <NyxidIcon className="mx-auto h-8 w-8" />
        {failed ? (
          <>
            <h1 className="mt-4 text-[15px] font-semibold">
              Unable to start connection
            </h1>
            <p className="mt-2 text-[12px] leading-5 text-muted-foreground">
              Close this window and start the connection again from NyxID.
            </p>
          </>
        ) : (
          <>
            <Loader2 className="mx-auto mt-5 h-5 w-5 animate-spin text-muted-foreground" />
            <h1 className="mt-3 text-[15px] font-semibold">Connecting...</h1>
            <p className="mt-2 text-[12px] text-muted-foreground">
              Preparing secure authorization
            </p>
          </>
        )}
      </section>
    </main>
  );
}
