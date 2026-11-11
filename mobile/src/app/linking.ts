import {
  getStateFromPath,
  type LinkingOptions,
} from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import { Linking } from "react-native";
import type { RootStackParamList } from "./navigationTypes";
import { navigationLinkingConfig } from "./navigationLinking";
import { workspaceStateFromPath } from "./workspaceCanonicalLinks";
import { capture } from "../lib/telemetry";
import type { LinkType } from "../lib/telemetry-schema";
import {
  IS_DEV_BUILD,
  NYXID_APP_SCHEME,
  NYXID_TRUSTED_API_BASE_URLS,
  NYXID_TRUSTED_FRONTEND_URLS,
  NYXID_UNIVERSAL_LINK_HOST,
} from "../lib/env";
import { buildLinkingPrefixes } from "../lib/linkingPrefixes";

import {
  nativeNotificationUrl,
  notificationDestination,
} from "../lib/notifications/notificationDestination";

export const notificationLinkConfig = {
  scheme: NYXID_APP_SCHEME,
  trustedUrls: [
    ...NYXID_TRUSTED_FRONTEND_URLS,
    ...NYXID_TRUSTED_API_BASE_URLS,
    ...(NYXID_UNIVERSAL_LINK_HOST
      ? [`https://${NYXID_UNIVERSAL_LINK_HOST}`]
      : []),
  ],
};

const directLinkListeners = new Set<() => void>();

export function subscribeDirectLinkOpened(listener: () => void): () => void {
  directLinkListeners.add(listener);
  return () => {
    directLinkListeners.delete(listener);
  };
}

function reportDirectLinkOpened(url: string): void {
  // React Navigation owns direct links; supersede older queued notification
  // intent before it receives this destination. Ignore untrusted URL events.
  if (nativeNotificationUrl(url, notificationLinkConfig)) {
    for (const listener of directLinkListeners) listener();
  }
  reportDeepLinkOpened(url);
}

export function extractChallengeIdFromUrl(url: string): string | null {
  try {
    const normalized = nativeNotificationUrl(url, notificationLinkConfig);
    if (!normalized) return null;
    return new URL(normalized).searchParams.get("challengeId");
  } catch {
    return null;
  }
}

const extractUrlFromNotificationData = (data: unknown) =>
  notificationDestination(data, notificationLinkConfig);

export function extractUrlFromNotificationResponse(
  response: Notifications.NotificationResponse | null,
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

    const remoteMessageUrl = extractUrlFromNotificationData(
      pushTrigger.remoteMessage?.data,
    );
    if (remoteMessageUrl) {
      return remoteMessageUrl;
    }
  }

  return null;
}

export function extractChallengeIdFromNotificationResponse(
  response: Notifications.NotificationResponse | null,
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

// No telemetry buffer: pre-consent/pre-login events cannot acquire later
// permission or become associated with a newly selected account.
export function reportDeepLinkOpened(url: string): void {
  void capture({
    name: "mobile.deep_link_opened",
    props: { link_type: classifyLinkType(url) },
  });
}

/**
 * Transform challenge deep-link URLs (nyxid://challenge/{id}) into activity
 * URLs with a challengeId query param so React Navigation routes to the
 * Activity screen which opens the bottom sheet.
 */
function rewriteChallengeUrl(url: string): string {
  const challengeId = extractChallengeIdFromUrl(url);
  if (challengeId) {
    return `${NYXID_APP_SCHEME}://activity?challengeId=${encodeURIComponent(challengeId)}`;
  }
  return url;
}

export const appLinking: LinkingOptions<RootStackParamList> = {
  prefixes: buildLinkingPrefixes({
    appScheme: NYXID_APP_SCHEME,
    // Both embedded sets: native links can arrive on either API host.
    apiBaseUrls: NYXID_TRUSTED_API_BASE_URLS,
    frontendUrls: NYXID_TRUSTED_FRONTEND_URLS,
    universalLinkHost: NYXID_UNIVERSAL_LINK_HOST,
    allowHttp: IS_DEV_BUILD,
  }),
  config: navigationLinkingConfig,
  getStateFromPath: (path, options) =>
    workspaceStateFromPath(path, (next) => getStateFromPath(next, options)),
  async getInitialURL() {
    try {
      const directUrl = await Linking.getInitialURL();
      if (directUrl) {
        reportDirectLinkOpened(directUrl);
        return rewriteChallengeUrl(directUrl);
      }
    } catch (error) {
      if (__DEV__) console.warn("[linking] getInitialURL failed", error);
    }

    // App retains tapped notifications until authentication and navigation guards allow them.
    return null;
  },
  subscribe(listener) {
    let linkingSubscription: { remove: () => void } | null = null;

    try {
      linkingSubscription = Linking.addEventListener("url", ({ url }) => {
        reportDirectLinkOpened(url);
        listener(rewriteChallengeUrl(url));
      });
    } catch (error) {
      if (__DEV__) console.warn("[linking] url subscription failed", error);
    }

    return () => {
      linkingSubscription?.remove();
    };
  },
};
