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

const POPUP_READY_TIMEOUT_MS = 2_000;
export const OAUTH_PROVIDER_ORIGIN_KEY = "nyxid.oauth.provider-origin";

export interface OAuthPopupHandle {
  readonly launchId: string;
  readonly ready: Promise<void>;
  navigate(url: string, nonce: string): Promise<void>;
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
    async navigate(url, nonce) {
      if (!validateAuthorizationUrl(url, nonce)) {
        throw new Error("Invalid OAuth authorization URL");
      }
      await ready;
      popup.postMessage(
        { type: "oauth_launch_navigate", launchId, nonce, url },
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
