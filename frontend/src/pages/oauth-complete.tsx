import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, CheckCircle2, RotateCcw, X } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button } from "@/components/ui/button";
import {
  openOAuthChannel,
  postOAuthAction,
  postOAuthResult,
  readOAuthLaunchContext,
  writeOAuthLaunchContext,
} from "@/lib/oauth-popup";
import {
  isOAuthAckMessage,
  isOAuthRetryMessage,
  oauthCompletionSearchSchema,
  validateAuthorizationUrl,
} from "@/schemas/oauth-popup";
import type { OAuthActionMessage } from "@/types/oauth-popup";

const AUTO_CLOSE_MS = 3_000;
const ACK_TIMEOUT_MS = 400;
const RETRY_TIMEOUT_MS = 10_000;

const ERROR_COPY = {
  access_denied: [
    "Authorization declined",
    "No changes were made. You can try again when ready.",
  ],
  provider_error: [
    "Provider could not complete authorization",
    "Try the connection again.",
  ],
  state_expired: [
    "Authorization expired",
    "Start a fresh authorization attempt.",
  ],
  state_replayed: [
    "Authorization already used",
    "Start over to create a fresh request.",
  ],
  state_invalid: [
    "Authorization is no longer valid",
    "Return to NyxID and try again.",
  ],
  session_mismatch: [
    "Different NyxID account",
    "Return to the original NyxID tab and switch accounts before retrying.",
  ],
  session_required: [
    "NyxID session required",
    "Sign in to NyxID and start again.",
  ],
  exchange_failed: [
    "Connection could not be saved",
    "Try again. Your existing connection was not replaced.",
  ],
  server_error: [
    "Connection could not be completed",
    "Return to NyxID and try again.",
  ],
} as const;

export function OAuthCompletePage() {
  const [search] = useState(() =>
    oauthCompletionSearchSchema.parse(
      Object.fromEntries(new URLSearchParams(window.location.search)),
    ),
  );
  const [launchContext] = useState(() =>
    search.flow === "cc" ? readOAuthLaunchContext() : null,
  );
  const displayContext =
    search.flow === "cc" && launchContext?.nonce === search.nonce
      ? launchContext
      : null;
  const expectedProviderOrigin = displayContext?.providerOrigin ?? null;
  const [manualReturn, setManualReturn] = useState(false);
  const [staying, setStaying] = useState(false);
  const [pendingAction, setPendingAction] = useState<
    OAuthActionMessage["action"] | null
  >(null);
  const channelRef = useRef<BroadcastChannel | null>(null);
  const validCompletion = Boolean(search.status && search.flow && search.nonce);

  useEffect(() => {
    const channel = search.nonce ? openOAuthChannel(search.nonce) : null;
    channelRef.current = channel;
    if (channel && search.status) {
      postOAuthResult(channel, {
        type: "oauth_result",
        status: search.status,
        ...(search.flow ? { flow: search.flow } : {}),
        ...(search.code ? { code: search.code } : {}),
      });
    }
    window.history.replaceState(null, "", "/oauth-complete");
    return () => {
      channelRef.current = null;
      channel?.close();
    };
  }, [search]);

  useEffect(() => {
    if (search.status !== "complete" || staying) return;
    const cancel = () => setStaying(true);
    window.addEventListener("pointerdown", cancel, { once: true });
    window.addEventListener("keydown", cancel, { once: true });
    const timer = window.setTimeout(() => {
      window.close();
      window.setTimeout(() => setManualReturn(true), 300);
    }, AUTO_CLOSE_MS);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("pointerdown", cancel);
      window.removeEventListener("keydown", cancel);
    };
  }, [search.status, staying]);

  const sendAction = useCallback(
    (action: OAuthActionMessage["action"]) => {
      if (pendingAction) return;
      const channel = channelRef.current;
      if (!channel) {
        setManualReturn(true);
        return;
      }
      setPendingAction(action);
      postOAuthAction(channel, { type: "oauth_action", action });
      const timeout = window.setTimeout(
        () => {
          setPendingAction(null);
          setManualReturn(true);
        },
        action === "retry" ? RETRY_TIMEOUT_MS : ACK_TIMEOUT_MS,
      );
      const onMessage = (event: MessageEvent<unknown>) => {
        if (
          action === "retry" &&
          expectedProviderOrigin !== null &&
          isOAuthRetryMessage(event.data, expectedProviderOrigin)
        ) {
          const authorizationUrl = validateAuthorizationUrl(
            event.data.url,
            event.data.nextNonce,
            expectedProviderOrigin,
          );
          if (authorizationUrl === null) return;
          writeOAuthLaunchContext({
            ...displayContext,
            providerOrigin: authorizationUrl.origin,
            nonce: event.data.nextNonce,
          });
          window.clearTimeout(timeout);
          channel.removeEventListener("message", onMessage);
          channel.close();
          window.location.assign(authorizationUrl.href);
        } else if (action !== "retry" && isOAuthAckMessage(event.data)) {
          window.clearTimeout(timeout);
          channel.removeEventListener("message", onMessage);
          window.close();
          window.setTimeout(() => setManualReturn(true), 300);
        }
      };
      channel.addEventListener("message", onMessage);
    },
    [displayContext, expectedProviderOrigin, pendingAction],
  );

  if (!validCompletion) {
    return (
      <OAuthResultShell icon="neutral" title="Nothing to complete">
        <p>
          This window isn't part of an active connection attempt. If you were
          connecting a service, its status appears in your NyxID chat — this
          window can be closed.
        </p>
        <Button asChild className="mt-5 w-full">
          <a href="/">Back to NyxID</a>
        </Button>
      </OAuthResultShell>
    );
  }

  const success = search.status === "complete";
  const errorCopy = search.code ? ERROR_COPY[search.code] : undefined;
  const retryable =
    search.code !== "session_mismatch" &&
    search.code !== "session_required" &&
    search.code !== "state_invalid";
  const canRetryHere = retryable && expectedProviderOrigin !== null;
  const serviceName = displayContext?.serviceName;
  const title = success
    ? "Authorization response received"
    : (errorCopy?.[0] ?? "Authorization failed");

  return (
    <OAuthResultShell
      icon={success ? "neutral" : "error"}
      title={!success && serviceName ? `${serviceName}: ${title}` : title}
    >
      <p role="status" aria-live="polite">
        {success
          ? serviceName
            ? `Return to your NyxID chat — the ${serviceName} connection's status appears there and updates automatically.`
            : "Return to NyxID while the connection is verified."
          : (errorCopy?.[1] ?? "Return to NyxID and try again.")}
      </p>
      {!success && retryable && expectedProviderOrigin === null ? (
        <div className="mt-5 rounded-md border border-border bg-muted/40 px-3 py-2 text-[12px] text-foreground">
          Return to your NyxID tab and start the connection again there.
        </div>
      ) : null}
      {manualReturn ? (
        <div className="mt-5 rounded-md border border-border bg-muted/40 px-3 py-2 text-[12px] text-foreground">
          Return to your NyxID tab to continue. You can close this window.
        </div>
      ) : (
        <div className="mt-5 flex flex-col gap-2">
          {success ? (
            <Button
              autoFocus
              disabled={pendingAction !== null}
              onClick={() => sendAction("view_result")}
            >
              View connection
            </Button>
          ) : canRetryHere ? (
            <Button
              autoFocus
              disabled={pendingAction !== null}
              onClick={() => sendAction("retry")}
            >
              <RotateCcw /> Try again
            </Button>
          ) : null}
          <Button
            variant="ghost"
            disabled={pendingAction !== null}
            onClick={() => {
              if (success) setStaying(true);
              else sendAction("dismiss");
            }}
          >
            {success ? (
              "Stay here"
            ) : (
              <>
                <X /> Close
              </>
            )}
          </Button>
        </div>
      )}
    </OAuthResultShell>
  );
}

function OAuthResultShell({
  icon,
  title,
  children,
}: {
  readonly icon: "success" | "error" | "neutral";
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  return (
    <main className="flex min-h-screen items-center justify-center bg-background px-6 text-foreground">
      <section className="w-full max-w-sm rounded-lg border border-border bg-card p-6 shadow-lg">
        <div className="flex items-center gap-3">
          <NyxidIcon className="h-8 w-8 shrink-0" />
          {icon === "success" ? (
            <CheckCircle2 className="h-5 w-5 text-success" />
          ) : icon === "error" ? (
            <AlertCircle className="h-5 w-5 text-destructive" />
          ) : null}
        </div>
        <h1 className="mt-5 text-[17px] font-semibold">{title}</h1>
        <div className="mt-2 text-[12px] leading-5 text-muted-foreground">
          {children}
        </div>
      </section>
    </main>
  );
}
