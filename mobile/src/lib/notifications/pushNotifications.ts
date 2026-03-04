import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import { Platform } from "react-native";
import { mobileApi } from "../api/mobileApi";

type PushActivateResult = {
  permission: "granted" | "denied";
  token: string | null;
  registered: boolean;
  mode: "registered" | "rotated" | "unchanged" | "none";
  reason?: "permission_denied" | "token_unavailable" | "register_failed";
};

const PUSH_TOKEN_STORE_KEY = "nyxid.push.device_token";

let isNotificationHandlerConfigured = false;

function configureNotificationHandler() {
  if (isNotificationHandlerConfigured) return;

  Notifications.setNotificationHandler({
    handleNotification: async () => ({
      shouldShowAlert: true,
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: false,
      shouldSetBadge: false,
    }),
  });

  isNotificationHandlerConfigured = true;
}

async function ensureAndroidChannel() {
  if (Platform.OS !== "android") return;

  await Notifications.setNotificationChannelAsync("default", {
    name: "default",
    importance: Notifications.AndroidImportance.HIGH,
    vibrationPattern: [0, 200, 200, 200],
    lightColor: "#8B5CF6",
  });
}

async function ensureNotificationPermission(): Promise<"granted" | "denied"> {
  const current = await Notifications.getPermissionsAsync();
  if (current.status === "granted") {
    return "granted";
  }

  const requested = await Notifications.requestPermissionsAsync();
  return requested.status === "granted" ? "granted" : "denied";
}

function resolvePlatform(): "ios" | "android" | "web" | "unknown" {
  if (Platform.OS === "ios") return "ios";
  if (Platform.OS === "android") return "android";
  if (Platform.OS === "web") return "web";
  return "unknown";
}

function resolveProvider(platform: "ios" | "android" | "web" | "unknown"): "apns" | "fcm" {
  if (platform === "ios") return "apns";
  return "fcm";
}

function normalizeDeviceToken(token: Notifications.DevicePushToken): string | null {
  if (typeof token.data === "string" && token.data.length > 0) {
    return token.data;
  }

  if (token.data && typeof token.data === "object") {
    const maybeToken = (token.data as { token?: unknown }).token;
    if (typeof maybeToken === "string" && maybeToken.length > 0) {
      return maybeToken;
    }
  }

  return null;
}

export async function initializeNotificationRuntime(): Promise<() => void> {
  configureNotificationHandler();
  await ensureAndroidChannel();
  return () => {};
}

export async function activatePushAfterLogin(): Promise<PushActivateResult> {
  const permission = await ensureNotificationPermission();
  if (permission !== "granted") {
    return {
      permission,
      token: null,
      registered: false,
      mode: "none",
      reason: "permission_denied",
    };
  }

  let token: string | null = null;
  try {
    const devicePushToken = await Notifications.getDevicePushTokenAsync();
    token = normalizeDeviceToken(devicePushToken);
  } catch (error) {
    if (__DEV__) console.warn("[push] native push token unavailable", error);
    return {
      permission,
      token: null,
      registered: false,
      mode: "none",
      reason: "token_unavailable",
    };
  }

  if (!token) {
    return {
      permission,
      token: null,
      registered: false,
      mode: "none",
      reason: "token_unavailable",
    };
  }

  const previousToken = await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY);
  const platform = resolvePlatform();
  const provider = resolveProvider(platform);

  try {
    if (previousToken && previousToken !== token) {
      await mobileApi.rotatePushToken({
        token,
        previous_token: previousToken,
        provider,
        platform,
      });
      await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, token);
      return {
        permission,
        token,
        registered: true,
        mode: "rotated",
      };
    }

    if (!previousToken) {
      await mobileApi.registerPushToken({
        token,
        provider,
        platform,
      });
      await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, token);
      return {
        permission,
        token,
        registered: true,
        mode: "registered",
      };
    }

    return {
      permission,
      token,
      registered: true,
      mode: "unchanged",
    };
  } catch (error) {
    if (__DEV__) console.warn("[push] register token failed", error);
    return {
      permission,
      token,
      registered: false,
      mode: "none",
      reason: "register_failed",
    };
  }
}
