import type { LinkingOptions } from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import { Linking } from "react-native";
import type { RootStackParamList } from "./AppNavigator";

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

export const appLinking: LinkingOptions<RootStackParamList> = {
  prefixes: ["nyxid://"],
  config: {
    screens: {
      Auth: "auth",
      Dashboard: "dashboard",
      AccountSettings: "account",
      Inbox: "inbox",
      ChallengeMinimal: "challenge/:challengeId/minimal",
      ChallengeOptions: "challenge/:challengeId/options",
      ChallengeDetail: "challenge/:challengeId",
      Approvals: "approvals",
      RevokeConfirm: "approvals/:approvalId/revoke",
      RevokeSuccess: "approvals/revoke-success",
      TermsOfService: "terms",
      PrivacyPolicy: "privacy",
    },
  },
  async getInitialURL() {
    try {
      const directUrl = await Linking.getInitialURL();
      if (directUrl) return directUrl;
    } catch (error) {
      if (__DEV__) console.warn("[linking] getInitialURL failed", error);
    }

    try {
      const lastNotificationResponse = await Notifications.getLastNotificationResponseAsync();
      return extractUrlFromNotificationResponse(lastNotificationResponse);
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
        listener(url);
      });
    } catch (error) {
      if (__DEV__) console.warn("[linking] url subscription failed", error);
    }

    try {
      notificationSubscription =
        Notifications.addNotificationResponseReceivedListener((response) => {
          const url = extractUrlFromNotificationResponse(response);
          if (url) listener(url);
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
