import type { LinkingOptions } from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import { Linking } from "react-native";
import type { RootStackParamList } from "./AppNavigator";
import { capture } from "../lib/telemetry";
import type { LinkType } from "../lib/telemetry-schema";

type NotificationData = {
  deeplink?: unknown;
  deep_link?: unknown;
  url?: unknown;
  link?: unknown;
  challenge_id?: unknown;
  challengeId?: unknown;
  request_id?: unknown;
  data?: unknown;
  payload?: unknown;
  dataString?: unknown;
};

function buildChallengeUrl(challengeId: string): string {
  return `nyxid://challenge/${encodeURIComponent(challengeId)}`;
}

export function extractChallengeIdFromUrl(url: string): string | null {
  const match = url.match(/(?:^|\/\/|\/)challenge\/([^/?#]+)/i);
  if (!match || !match[1]) return null;

  try {
    const decoded = decodeURIComponent(match[1]);
    return decoded.trim().length > 0 ? decoded.trim() : null;
  } catch {
    return match[1];
  }
}

function asNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function extractUrlFromNotificationData(data: unknown): string | null {
  const pending: unknown[] = [data];
  const visited = new Set<object>();

  while (pending.length > 0) {
    const current = pending.shift();
    if (!current || typeof current !== "object") {
      continue;
    }
    if (visited.has(current)) {
      continue;
    }
    visited.add(current);

    const payload = current as NotificationData;
    const deeplink =
      asNonEmptyString(payload.deeplink) ?? asNonEmptyString(payload.deep_link);
    if (deeplink) return deeplink;

    const url = asNonEmptyString(payload.url) ?? asNonEmptyString(payload.link);
    if (url) return url;

    const challengeId =
      asNonEmptyString(payload.challenge_id) ??
      asNonEmptyString(payload.challengeId) ??
      asNonEmptyString(payload.request_id);
    if (challengeId) {
      return buildChallengeUrl(challengeId);
    }

    if (typeof payload.dataString === "string" && payload.dataString.length > 0) {
      try {
        pending.push(JSON.parse(payload.dataString));
      } catch {
        // ignore invalid JSON and continue
      }
    }

    for (const value of Object.values(payload)) {
      if (!value) {
        continue;
      }

      if (typeof value === "object") {
        pending.push(value);
        continue;
      }

      if (
        typeof value === "string" &&
        value.length > 1 &&
        (value.startsWith("{") || value.startsWith("["))
      ) {
        try {
          pending.push(JSON.parse(value));
        } catch {
          // ignore non-JSON string fields
        }
      }
    }
  }

  return null;
}

export function extractUrlFromNotificationResponse(
  response: Notifications.NotificationResponse | null
): string | null {
  if (!response) return null;

  const request = response.notification.request;

  const contentUrl = extractUrlFromNotificationData(request.content.data);
  if (contentUrl) {
    return contentUrl;
  }

  const trigger = request.trigger as unknown;
  if (trigger && typeof trigger === "object") {
    const pushTrigger = trigger as {
      type?: unknown;
      payload?: unknown;
      remoteMessage?: { data?: unknown } | null;
    };

    if (pushTrigger.type !== "push") {
      return null;
    }

    const pushPayloadUrl = extractUrlFromNotificationData(pushTrigger.payload);
    if (pushPayloadUrl) {
      return pushPayloadUrl;
    }

    const remoteMessageUrl = extractUrlFromNotificationData(pushTrigger.remoteMessage?.data);
    if (remoteMessageUrl) {
      return remoteMessageUrl;
    }
  }

  return null;
}

export function extractChallengeIdFromNotificationResponse(
  response: Notifications.NotificationResponse | null
): string | null {
  const url = extractUrlFromNotificationResponse(response);
  if (!url) return null;
  return extractChallengeIdFromUrl(url);
}

/**
 * Classify a deep-link URL into a narrow enum for telemetry. Never emits
 * the raw URL or any token — `telemetry.ts` also scrubs deep-link URLs
 * at the vendor boundary, but we defense-in-depth by reporting only the
 * category here.
 */
function classifyLinkType(url: string): LinkType {
  const lower = url.toLowerCase();
  if (/(?:^|\/\/|\/)challenge\//.test(lower) || /\bchallengeid=/.test(lower)) {
    return "challenge";
  }
  if (/(?:^|\/\/|\/)(?:approve|approval|approvals)\b/.test(lower)) {
    return "approval";
  }
  return "other";
}

// Cold-start deep-link + push-open events fire from `getInitialURL()` before
// `AuthSessionContext` calls `initTelemetry()`. Without buffering, those
// captures land while the mobile client is still uninitialized and are
// silently dropped -- losing exactly the cold-start launches we want to
// measure. The buffer sits in one of three states so the same module can
// handle (a) initial cold start with pending consent hydration, (b) opted-in
// steady state, and (c) in-session consent toggles without leaking pre-consent
// events on re-enable.
//
//   "pending"   — consent not yet known or consent-on but init not finished.
//                 Buffer new events up to a hard cap.
//   "active"    — init complete. Pass new events straight to capture().
//   "discarded" — consent explicitly off. Drop new events immediately
//                 without buffering. No state accumulates.
//
// Transitions:
//   pending -> active:    flushPendingDeepLinks() drains + marks active.
//   * -> discarded:       discardPendingDeepLinks() clears + marks discarded.
//   discarded -> pending: armDeepLinkBuffer() (called when consent flips
//                          back on, BEFORE initTelemetry runs) so the brief
//                          window between consent=on and init-complete is
//                          captured instead of dropped.
//
// Hard cap keeps unbounded growth impossible even in pathological cases.
type DeepLinkBufferState = "pending" | "active" | "discarded";
let deepLinkBufferState: DeepLinkBufferState = "pending";
const MAX_PENDING_DEEP_LINKS = 16;
const pendingDeepLinkTypes: Array<{ link_type: LinkType }> = [];

function reportDeepLinkOpened(url: string): void {
  const props = { link_type: classifyLinkType(url) };
  switch (deepLinkBufferState) {
    case "active":
      try {
        capture({ name: "mobile.deep_link_opened", props });
      } catch {
        // telemetry must never break the linking pipeline
      }
      return;
    case "pending":
      if (pendingDeepLinkTypes.length < MAX_PENDING_DEEP_LINKS) {
        pendingDeepLinkTypes.push(props);
      }
      return;
    case "discarded":
      // Consent is off; drop silently.
      return;
  }
}

/**
 * Called once per init cycle, immediately after `initTelemetry()` resolves,
 * to drain deep-link events captured while the client was warming up.
 * Idempotent: if state is already "active" this is a no-op. Callers MUST
 * NOT invoke this unless the user has consented to telemetry;
 * `initTelemetry()` itself short-circuits on !consent, so tying the flush
 * to init resolution satisfies the consent boundary.
 */
export function flushPendingDeepLinks(): void {
  if (deepLinkBufferState === "active") return;
  deepLinkBufferState = "active";
  for (const props of pendingDeepLinkTypes) {
    try {
      capture({ name: "mobile.deep_link_opened", props });
    } catch {
      // telemetry must never break the linking pipeline
    }
  }
  pendingDeepLinkTypes.length = 0;
}

/**
 * Called when telemetry is explicitly off (consent=false / revoked / no
 * DSN configured). Clears the buffer and moves to the "discarded" state
 * so subsequent events are dropped silently instead of accumulating.
 * Prevents unbounded queue growth on installs that never opt in and
 * keeps pre-consent events from leaking if the user later re-enables.
 */
export function discardPendingDeepLinks(): void {
  deepLinkBufferState = "discarded";
  pendingDeepLinkTypes.length = 0;
}

/**
 * Called by the consent-on branch of `AuthSessionContext` BEFORE
 * `initTelemetry()` runs, to re-arm buffering if we had previously
 * discarded. Brings the state back to "pending" so deep links that
 * arrive during the init promise are captured instead of dropped.
 * No-op when state is already "pending" or "active" -- this is the only
 * transition from "discarded" back to "pending", so it can't leak
 * pre-consent events.
 */
export function armDeepLinkBuffer(): void {
  if (deepLinkBufferState === "discarded") {
    deepLinkBufferState = "pending";
  }
}

/**
 * Transform challenge deep-link URLs (nyxid://challenge/{id}) into activity
 * URLs with a challengeId query param so React Navigation routes to the
 * Activity screen which opens the bottom sheet.
 */
function rewriteChallengeUrl(url: string): string {
  const challengeId = extractChallengeIdFromUrl(url);
  if (challengeId) {
    return `nyxid://activity?challengeId=${encodeURIComponent(challengeId)}`;
  }
  return url;
}

export const appLinking: LinkingOptions<RootStackParamList> = {
  prefixes: ["nyxid://"],
  config: {
    screens: {
      Auth: "auth",
      Activity: {
        path: "activity",
        parse: { challengeId: String },
      },
      AccountSettings: "account",
      TermsOfService: "terms",
      PrivacyPolicy: "privacy",
    },
  },
  async getInitialURL() {
    try {
      const directUrl = await Linking.getInitialURL();
      if (directUrl) {
        reportDeepLinkOpened(directUrl);
        return rewriteChallengeUrl(directUrl);
      }
    } catch (error) {
      if (__DEV__) console.warn("[linking] getInitialURL failed", error);
    }

    try {
      const lastNotificationResponse = await Notifications.getLastNotificationResponseAsync();
      const url = extractUrlFromNotificationResponse(lastNotificationResponse);
      if (url) {
        reportDeepLinkOpened(url);
        return rewriteChallengeUrl(url);
      }
      return null;
    } catch (error) {
      if (__DEV__) {
        console.warn("[linking] getLastNotificationResponseAsync failed", error);
      }
      return null;
    }
  },
  subscribe(listener) {
    let linkingSubscription: { remove: () => void } | null = null;
    let notificationSubscription: { remove: () => void } | null = null;

    try {
      linkingSubscription = Linking.addEventListener("url", ({ url }) => {
        reportDeepLinkOpened(url);
        listener(rewriteChallengeUrl(url));
      });
    } catch (error) {
      if (__DEV__) console.warn("[linking] url subscription failed", error);
    }

    try {
      notificationSubscription =
        Notifications.addNotificationResponseReceivedListener((response) => {
          const url = extractUrlFromNotificationResponse(response);
          if (url) {
            reportDeepLinkOpened(url);
            listener(rewriteChallengeUrl(url));
          }
        });
    } catch (error) {
      if (__DEV__) console.warn("[linking] notification subscription failed", error);
    }

    return () => {
      linkingSubscription?.remove();
      notificationSubscription?.remove();
    };
  },
};
