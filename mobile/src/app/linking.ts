import type { LinkingOptions } from "@react-navigation/native";
import * as Notifications from "expo-notifications";
import { Linking } from "react-native";
import type { RootStackParamList } from "./AppNavigator";

type NotificationData = {
  deeplink?: unknown;
  url?: unknown;
  challenge_id?: unknown;
  challengeId?: unknown;
  request_id?: unknown;
};

function buildChallengeUrl(challengeId: string): string {
  return `nyxid://challenge/${encodeURIComponent(challengeId)}`;
}

function extractUrlFromNotificationResponse(
  response: Notifications.NotificationResponse | null
): string | null {
  if (!response) return null;

  const data = (response.notification.request.content.data ?? {}) as NotificationData;

  if (typeof data.deeplink === "string" && data.deeplink.length > 0) {
    return data.deeplink;
  }
  if (typeof data.url === "string" && data.url.length > 0) {
    return data.url;
  }
  if (typeof data.challenge_id === "string" && data.challenge_id.length > 0) {
    return buildChallengeUrl(data.challenge_id);
  }
  if (typeof data.challengeId === "string" && data.challengeId.length > 0) {
    return buildChallengeUrl(data.challengeId);
  }
  if (typeof data.request_id === "string" && data.request_id.length > 0) {
    return buildChallengeUrl(data.request_id);
  }

  return null;
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
    const directUrl = await Linking.getInitialURL();
    if (directUrl) return directUrl;

    const lastNotificationResponse = await Notifications.getLastNotificationResponseAsync();
    return extractUrlFromNotificationResponse(lastNotificationResponse);
  },
  subscribe(listener) {
    const linkingSubscription = Linking.addEventListener("url", ({ url }) => {
      listener(url);
    });

    const notificationSubscription =
      Notifications.addNotificationResponseReceivedListener((response) => {
        const url = extractUrlFromNotificationResponse(response);
        if (url) listener(url);
      });

    return () => {
      linkingSubscription.remove();
      notificationSubscription.remove();
    };
  },
};
