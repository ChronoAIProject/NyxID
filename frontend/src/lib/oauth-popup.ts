import {
  oauthAttemptNonceSchema,
  validateAuthorizationUrl,
} from "@/schemas/oauth-popup";
import type {
  OAuthAckMessage,
  OAuthActionMessage,
  OAuthLaunchReadyMessage,
  OAuthResultMessage,
  OAuthRetryMessage,
} from "@/types/oauth-popup";

// Allow a cold/no-cache SPA interstitial to load. Expiry only switches the
// dialog to its non-destructive manual-link fallback.
const POPUP_READY_TIMEOUT_MS = 5_000;
const MAX_SERVICE_NAME_LENGTH = 64;
export const OAUTH_LAUNCH_CONTEXT_KEY = "nyxid.oauth.launch-context";

export interface PopupLaunchMetadata {
  readonly providerOrigin: string;
  readonly correlationId: string;
  readonly serviceName?: string;
}

export interface OAuthPopupHandle {
  readonly launchId: string;
  readonly ready: Promise<void>;
  navigate(url: string, nonce: string, serviceName?: string): Promise<void>;
  close(): void;
  /** A soft hint only: COOP can make a live popup appear closed. */
  isClosed(): boolean;
}

export function oauthChannelName(nonce: string): string | null {
  return oauthAttemptNonceSchema.safeParse(nonce).success
    ? `nyxid.oauth.${nonce}`
    : null;
}

export function openOAuthChannel(nonce: string): BroadcastChannel | null {
  const name = oauthChannelName(nonce);
  if (!name || typeof BroadcastChannel === "undefined") return null;
  return new BroadcastChannel(name);
}

function validServiceName(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= MAX_SERVICE_NAME_LENGTH
  );
}

export function writePopupLaunchMetadata(metadata: PopupLaunchMetadata): void {
  sessionStorage.setItem(OAUTH_LAUNCH_CONTEXT_KEY, JSON.stringify(metadata));
}

export function readPopupLaunchMetadata(): PopupLaunchMetadata | null {
  const raw = sessionStorage.getItem(OAUTH_LAUNCH_CONTEXT_KEY);
  if (raw === null) return null;
  try {
    const value = JSON.parse(raw) as unknown;
    if (typeof value !== "object" || value === null) return null;
    const record = value as Record<string, unknown>;
    if (
      typeof record.providerOrigin !== "string" ||
      typeof record.correlationId !== "string" ||
      !oauthAttemptNonceSchema.safeParse(record.correlationId).success ||
      (record.serviceName !== undefined &&
        !validServiceName(record.serviceName))
    ) {
      return null;
    }
    const providerUrl = new URL(record.providerOrigin);
    if (
      (providerUrl.protocol !== "https:" && providerUrl.protocol !== "http:") ||
      providerUrl.username !== "" ||
      providerUrl.password !== "" ||
      providerUrl.origin !== record.providerOrigin
    ) {
      return null;
    }
    return {
      providerOrigin: providerUrl.origin,
      correlationId: record.correlationId,
      ...(record.serviceName !== undefined
        ? { serviceName: record.serviceName }
        : {}),
    };
  } catch {
    return null;
  }
}

export function popupLaunchMetadataForNavigation(
  providerUrl: URL,
  correlationId: string,
  serviceName: unknown,
): PopupLaunchMetadata {
  return {
    providerOrigin: providerUrl.origin,
    correlationId,
    ...(validServiceName(serviceName) ? { serviceName } : {}),
  };
}

export function openOAuthPopup(): OAuthPopupHandle | null {
  const launchId = crypto.randomUUID();
  const popup = window.open(
    "/oauth-launching",
    `nyxid_oauth_${launchId}`,
    "popup,width=760,height=820",
  );
  if (!popup) return null;

  let settled = false;
  let resolveReady: (() => void) | undefined;
  let rejectReady: ((reason: Error) => void) | undefined;
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  void ready.catch(() => undefined);
  const onMessage = (event: MessageEvent<unknown>) => {
    const message = event.data as Partial<OAuthLaunchReadyMessage> | null;
    if (
      event.origin !== window.location.origin ||
      event.source !== popup ||
      message?.type !== "oauth_launch_ready" ||
      message.launchId !== launchId
    ) {
      return;
    }
    settled = true;
    window.clearTimeout(timeoutId);
    window.removeEventListener("message", onMessage);
    resolveReady?.();
  };
  window.addEventListener("message", onMessage);
  const timeoutId = window.setTimeout(() => {
    if (settled) return;
    settled = true;
    window.removeEventListener("message", onMessage);
    rejectReady?.(new Error("OAuth popup did not become ready"));
  }, POPUP_READY_TIMEOUT_MS);

  return {
    launchId,
    ready,
    async navigate(url, nonce, serviceName) {
      const authorizationUrl = validateAuthorizationUrl(url, nonce);
      if (authorizationUrl === null) {
        throw new Error("Invalid OAuth authorization URL");
      }
      await ready;
      popup.postMessage(
        {
          type: "oauth_launch_navigate",
          launchId,
          nonce,
          url: authorizationUrl.href,
          ...(serviceName !== undefined ? { serviceName } : {}),
        },
        window.location.origin,
      );
    },
    close() {
      window.clearTimeout(timeoutId);
      window.removeEventListener("message", onMessage);
      if (!settled) {
        settled = true;
        rejectReady?.(
          new Error("OAuth popup was closed before it became ready"),
        );
      }
      try {
        popup.close();
      } catch {
        // A cross-origin or COOP-isolated popup may no longer be reachable.
      }
    },
    isClosed() {
      try {
        return popup.closed;
      } catch {
        return false;
      }
    },
  };
}

export function postOAuthResult(
  channel: BroadcastChannel,
  message: OAuthResultMessage,
): void {
  channel.postMessage(message);
}

export function postOAuthAction(
  channel: BroadcastChannel,
  message: OAuthActionMessage,
): void {
  channel.postMessage(message);
}

export function postOAuthAck(channel: BroadcastChannel): void {
  const message: OAuthAckMessage = { type: "oauth_ack" };
  channel.postMessage(message);
}

export function postOAuthRetry(
  channel: BroadcastChannel,
  message: OAuthRetryMessage,
): void {
  channel.postMessage(message);
}
