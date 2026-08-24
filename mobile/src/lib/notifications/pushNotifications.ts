import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import * as TaskManager from "expo-task-manager";
import { AppState, Platform } from "react-native";
import { mobileApi } from "../api/mobileApi";

type PushActivateResult = {
  permission: "granted" | "denied";
  token: string | null;
  registered: boolean;
  mode: "registered" | "rotated" | "unchanged" | "none";
  reason?: "permission_denied" | "token_unavailable" | "register_failed";
};

type PushActivateOptions = {
  forceRegister?: boolean;
};

const PUSH_TOKEN_STORE_KEY = "nyxid.push.device_token";
const PUSH_PENDING_SYNC_SIGNAL_KEY = "nyxid.push.pending_sync_signal";
const BACKGROUND_NOTIFICATION_TASK = "NYXID_BACKGROUND_NOTIFICATION_TASK";

type PushSyncSource = "foreground" | "background";
export type PushSyncSignalType =
  | "approval_request"
  | "approval_decision"
  | "approval_expired";

export type PushSyncSignal = {
  type: PushSyncSignalType;
  requestId: string;
  challengeId: string;
  decision?: string;
  source: PushSyncSource;
};

type PushSyncHandler = (signal: PushSyncSignal) => void;

let pushSyncHandler: PushSyncHandler | null = null;

let isNotificationHandlerConfigured = false;
let isBackgroundTaskRegistered = false;
let isBootstrapped = false;

const NOTIFICATION_DEDUPE_WINDOW_MS = 2000;
const lastShownAtByNotificationKey = new Map<string, number>();

/**
 * Behaviour returned when a notification must not surface as an OS alert.
 * Also keeps it out of Notification Center: an approval the user has already
 * seen (or acted on) in-app should not linger in the shade as a stale entry.
 */
const SUPPRESS_OS_NOTIFICATION = {
  shouldShowAlert: false,
  shouldShowBanner: false,
  shouldShowList: false,
  shouldPlaySound: false,
  shouldSetBadge: false,
} as const;

const PRESENT_OS_NOTIFICATION = {
  shouldShowAlert: true,
  shouldShowBanner: true,
  shouldShowList: true,
  shouldPlaySound: true,
  shouldSetBadge: false,
} as const;

/**
 * Notification content that arrived while the app was in front of the user.
 * The OS banner for it was suppressed, so the app owes them an in-app
 * equivalent -- the app shell renders it through `ToastOverlay`.
 */
export type ForegroundNotification = {
  title: string | null;
  body: string | null;
  challengeId: string | null;
  type: PushSyncSignalType | null;
};

type ForegroundNotificationHandler = (notification: ForegroundNotification) => void;

let foregroundNotificationHandler: ForegroundNotificationHandler | null = null;

/**
 * Registered by the app shell. Unlike `setPushSyncHandler`, there is no
 * persist-for-later fallback: an in-app toast is only meaningful while the
 * app is actually on screen, and the cache refresh + Activity list already
 * cover the "user comes back later" case.
 */
export function setForegroundNotificationHandler(
  handler: ForegroundNotificationHandler
): () => void {
  foregroundNotificationHandler = handler;
  return () => {
    if (foregroundNotificationHandler === handler) {
      foregroundNotificationHandler = null;
    }
  };
}

// The approval the user currently has open (detail sheet or detail screen).
// Set by those screens; `null` whenever nothing specific is being viewed.
let activeChallengeId: string | null = null;

export function setActiveChallengeId(challengeId: string | null): void {
  activeChallengeId = challengeId;
}

function pruneNotificationDedupeKeys(now: number): void {
  for (const [key, shownAt] of lastShownAtByNotificationKey) {
    if (now - shownAt >= NOTIFICATION_DEDUPE_WINDOW_MS) {
      lastShownAtByNotificationKey.delete(key);
    }
  }
}

/**
 * Decide whether a notification arriving right now should be shown by the OS.
 *
 * `handleNotification` is only consulted while the JS runtime is live -- in
 * practice, while the app is foregrounded. Background and killed delivery is
 * rendered by the OS without asking us, so nothing here can suppress those.
 */
function configureNotificationHandler() {
  if (isNotificationHandlerConfigured) return;

  Notifications.setNotificationHandler({
    handleNotification: async (notification) => {
      const content = notification.request.content;

      // Data-only pushes carry no user-visible copy; showing them yields an
      // empty alert.
      const isSilent = !content.title && !content.body;
      if (isSilent) {
        return SUPPRESS_OS_NOTIFICATION;
      }

      // The app is open and in front of the user. An OS banner here would
      // cover the very screen that is already reacting to this event (the
      // Activity list refreshes off the same push). Suppress it and let the
      // in-app toast present it instead, which is also what the platform
      // conventions expect. Note this deliberately tests `active` and not
      // merely "not background": during a transition (`inactive` -- app
      // switcher, Control Centre pulled down, incoming call) the user is not
      // looking at our UI, so the OS banner is still the right surface.
      if (AppState.currentState === "active") {
        return SUPPRESS_OS_NOTIFICATION;
      }

      const signal = parsePushSyncSignalFromData(content.data, "foreground");
      const notificationKey =
        (signal ? `${signal.type}:${signal.requestId}` : null) ??
        (notification.request.identifier ? `id:${notification.request.identifier}` : null) ??
        "unknown";

      const now = Date.now();
      const lastShownAt = lastShownAtByNotificationKey.get(notificationKey);
      if (lastShownAt && now - lastShownAt < NOTIFICATION_DEDUPE_WINDOW_MS) {
        return SUPPRESS_OS_NOTIFICATION;
      }
      pruneNotificationDedupeKeys(now);
      lastShownAtByNotificationKey.set(notificationKey, now);

      return PRESENT_OS_NOTIFICATION;
    },
  });

  isNotificationHandlerConfigured = true;
}

/**
 * Stand-in for the OS banner we suppressed above. Called for every
 * notification received while the runtime is live; bails out unless the app
 * really is on screen, so a backgrounded-but-live runtime does not end up
 * showing both an OS banner and an in-app one.
 */
function presentForegroundNotification(
  content: Notifications.NotificationContent,
  signal: PushSyncSignal | null
): void {
  if (AppState.currentState !== "active") return;

  const title = asNonEmptyString(content.title);
  const body = asNonEmptyString(content.body);
  if (!title && !body) return;

  // The user already has this exact approval open. Announcing it on top of
  // the screen they are acting on is pure noise.
  if (signal && activeChallengeId && signal.challengeId === activeChallengeId) {
    return;
  }

  foregroundNotificationHandler?.({
    title,
    body,
    challengeId: signal?.challengeId ?? null,
    type: signal?.type ?? null,
  });
}

/**
 * Must be called at module-load time (index.ts), BEFORE registerRootComponent.
 * Sets up notification handler + background task + Android channels so that
 * FCM messages arriving when the app is killed still produce visible alerts.
 */
export function bootstrapNotificationInfrastructure() {
  if (isBootstrapped) return;
  isBootstrapped = true;

  configureNotificationHandler();
  ensureBackgroundTaskDefined();
  void ensureAndroidChannels();
}

async function ensureAndroidChannels() {
  if (Platform.OS !== "android") return;

  await Notifications.setNotificationChannelAsync("default", {
    name: "Default",
    importance: Notifications.AndroidImportance.MAX,
    vibrationPattern: [0, 200, 200, 200],
    lightColor: "#9775fa",
    sound: "default",
    enableVibrate: true,
    showBadge: true,
  });

  await Notifications.setNotificationChannelAsync("approvals", {
    name: "Approvals",
    description: "Approval requests and decisions",
    importance: Notifications.AndroidImportance.MAX,
    vibrationPattern: [0, 200, 200, 200],
    lightColor: "#9775fa",
    sound: "default",
    enableVibrate: true,
    showBadge: true,
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

function asNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function parsePushSyncSignalFromData(
  data: unknown,
  source: PushSyncSource
): PushSyncSignal | null {
  if (!data || typeof data !== "object") {
    return null;
  }

  const payload = data as {
    type?: unknown;
    request_id?: unknown;
    challenge_id?: unknown;
    challengeId?: unknown;
    decision?: unknown;
  };

  const typeRaw = asNonEmptyString(payload.type);
  const requestId =
    asNonEmptyString(payload.request_id) ??
    asNonEmptyString(payload.challenge_id) ??
    asNonEmptyString(payload.challengeId);
  const challengeId =
    asNonEmptyString(payload.challenge_id) ??
    asNonEmptyString(payload.challengeId) ??
    requestId;

  if (!requestId || !challengeId) {
    return null;
  }

  const allowedTypes = ["approval_request", "approval_decision", "approval_expired"];
  if (typeRaw && !allowedTypes.includes(typeRaw)) {
    return null;
  }

  const type: PushSyncSignalType =
    typeRaw === "approval_decision"
      ? "approval_decision"
      : typeRaw === "approval_expired"
        ? "approval_expired"
        : "approval_request";

  return {
    type,
    requestId,
    challengeId,
    decision: asNonEmptyString(payload.decision) ?? undefined,
    source,
  };
}

async function persistPendingPushSyncSignal(signal: PushSyncSignal): Promise<void> {
  try {
    await SecureStore.setItemAsync(
      PUSH_PENDING_SYNC_SIGNAL_KEY,
      JSON.stringify({
        type: signal.type,
        request_id: signal.requestId,
        challenge_id: signal.challengeId,
        decision: signal.decision,
        source: signal.source,
      })
    );
  } catch (error) {
    if (__DEV__) console.warn("[push] persist pending sync signal failed", error);
  }
}

async function emitOrPersistPushSyncSignal(signal: PushSyncSignal): Promise<void> {
  if (pushSyncHandler) {
    pushSyncHandler(signal);
    return;
  }
  await persistPendingPushSyncSignal(signal);
}

function parsePushSyncSignalFromTaskPayload(
  payload: Notifications.NotificationTaskPayload
): PushSyncSignal | null {
  if ("notification" in payload && payload.notification && typeof payload.notification === "object") {
    const maybeNotificationData = (
      payload.notification as {
        request?: {
          content?: {
            data?: unknown;
          };
        };
      }
    ).request?.content?.data;

    if (maybeNotificationData !== undefined) {
      const signal = parsePushSyncSignalFromData(maybeNotificationData, "background");
      if (signal) {
        return signal;
      }
    }
  }

  if ("data" in payload && payload.data && typeof payload.data === "object") {
    const dataPayload = payload.data as {
      dataString?: unknown;
      [key: string]: unknown;
    };

    if (typeof dataPayload.dataString === "string" && dataPayload.dataString.length > 0) {
      try {
        const parsedData = JSON.parse(dataPayload.dataString) as unknown;
        const signal = parsePushSyncSignalFromData(parsedData, "background");
        if (signal) {
          return signal;
        }
      } catch {
        // fall through
      }
    }

    return parsePushSyncSignalFromData(dataPayload, "background");
  }

  return null;
}

function ensureBackgroundTaskDefined() {
  if (Platform.OS === "web") return;
  if (TaskManager.isTaskDefined(BACKGROUND_NOTIFICATION_TASK)) return;

  TaskManager.defineTask<Notifications.NotificationTaskPayload>(
    BACKGROUND_NOTIFICATION_TASK,
    async ({ data, error }) => {
      if (error) {
        if (__DEV__) console.warn("[push] background task error", error);
        return;
      }

      if (!data) return;

      const signal = parsePushSyncSignalFromTaskPayload(data);
      if (!signal) return;
      await persistPendingPushSyncSignal(signal);
    }
  );
}

async function ensureBackgroundTaskRegistered() {
  if (Platform.OS === "web") return;
  if (isBackgroundTaskRegistered) return;

  ensureBackgroundTaskDefined();

  try {
    const alreadyRegistered = await TaskManager.isTaskRegisteredAsync(
      BACKGROUND_NOTIFICATION_TASK
    );
    if (!alreadyRegistered) {
      await Notifications.registerTaskAsync(BACKGROUND_NOTIFICATION_TASK);
    }
    isBackgroundTaskRegistered = true;
  } catch (error) {
    if (__DEV__) {
      console.warn("[push] background notification task registration failed", error);
    }
  }
}

function safeAddPushTokenListener(
  listener: (token: Notifications.DevicePushToken) => void
): Notifications.EventSubscription | null {
  try {
    const maybeAddListener = (
      Notifications as unknown as {
        addPushTokenListener?: (
          callback: (token: Notifications.DevicePushToken) => void
        ) => Notifications.EventSubscription;
      }
    ).addPushTokenListener;

    if (typeof maybeAddListener !== "function") {
      if (__DEV__) {
        console.warn("[push] addPushTokenListener unavailable in current runtime");
      }
      return null;
    }

    return maybeAddListener(listener);
  } catch (error) {
    if (__DEV__) console.warn("[push] addPushTokenListener failed", error);
    return null;
  }
}

export function setPushSyncHandler(handler: PushSyncHandler | null): () => void {
  pushSyncHandler = handler;
  return () => {
    if (pushSyncHandler === handler) {
      pushSyncHandler = null;
    }
  };
}

export async function consumePendingPushSyncSignal(): Promise<PushSyncSignal | null> {
  try {
    const raw = await SecureStore.getItemAsync(PUSH_PENDING_SYNC_SIGNAL_KEY);
    if (!raw) {
      return null;
    }

    await SecureStore.deleteItemAsync(PUSH_PENDING_SYNC_SIGNAL_KEY);

    const parsed = JSON.parse(raw) as unknown;
    const sourceRaw =
      parsed && typeof parsed === "object"
        ? asNonEmptyString((parsed as { source?: unknown }).source)
        : null;
    const source: PushSyncSource = sourceRaw === "foreground" ? "foreground" : "background";
    return parsePushSyncSignalFromData(parsed, source);
  } catch (error) {
    if (__DEV__) console.warn("[push] consume pending sync signal failed", error);
    return null;
  }
}

export async function initializeNotificationRuntime(): Promise<() => void> {
  let foregroundSubscription: Notifications.EventSubscription | null = null;
  let tokenSubscription: Notifications.EventSubscription | null = null;

  try {
    configureNotificationHandler();
    await ensureAndroidChannels();
    await ensureBackgroundTaskRegistered();

    foregroundSubscription = Notifications.addNotificationReceivedListener(
      (notification) => {
        const content = notification.request.content;
        const signal = parsePushSyncSignalFromData(content.data, "foreground");

        if (signal) {
          void emitOrPersistPushSyncSignal(signal);
        }

        // Runs for non-approval pushes too -- those have no sync signal but
        // still deserve the in-app toast that replaced their OS one.
        presentForegroundNotification(content, signal);
      }
    );

    tokenSubscription = safeAddPushTokenListener((devicePushToken) => {
      const token = normalizeDeviceToken(devicePushToken);
      if (!token) return;

      void syncTokenWithBackend(token, false).then((result) => {
        if (__DEV__) {
          console.log("[push] sync after runtime token refresh", result);
        }
      });
    });
  } catch (error) {
    if (__DEV__) console.warn("[push] initialize runtime failed", error);
  }

  return () => {
    foregroundSubscription?.remove();
    tokenSubscription?.remove();
  };
}

export async function clearLocalPushRegistrationState(): Promise<void> {
  try {
    await Promise.all([
      SecureStore.deleteItemAsync(PUSH_TOKEN_STORE_KEY),
      SecureStore.deleteItemAsync(PUSH_PENDING_SYNC_SIGNAL_KEY),
    ]);
  } catch (error) {
    if (__DEV__) console.warn("[push] clear local push registration state failed", error);
  }
}

export async function clearPendingPushSyncSignal(): Promise<void> {
  try {
    await SecureStore.deleteItemAsync(PUSH_PENDING_SYNC_SIGNAL_KEY);
  } catch (error) {
    if (__DEV__) console.warn("[push] clear pending sync signal failed", error);
  }
}

export async function deactivatePushOnLogout(): Promise<boolean> {
  const token = await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY);
  if (!token) {
    return true;
  }

  const platform = resolvePlatform();
  if (platform !== "ios" && platform !== "android") {
    return true;
  }

  const provider = resolveProvider(platform);
  try {
    await mobileApi.unregisterPushToken({
      token,
      provider,
      platform,
    });
    return true;
  } catch (error) {
    if (__DEV__) console.warn("[push] unregister on logout failed", error);
    return false;
  }
}

export async function activatePushAfterLogin(
  options: PushActivateOptions = {}
): Promise<PushActivateResult> {
  const forceRegister = options.forceRegister === true;
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

  return syncTokenWithBackend(token, forceRegister);
}

async function syncTokenWithBackend(
  token: string,
  forceRegister: boolean
): Promise<PushActivateResult> {
  const previousToken = await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY);
  const platform = resolvePlatform();
  if (platform !== "ios" && platform !== "android") {
    return {
      permission: "granted",
      token,
      registered: false,
      mode: "none",
      reason: "register_failed",
    };
  }
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
        permission: "granted",
        token,
        registered: true,
        mode: "rotated",
      };
    }

    if (!previousToken || forceRegister) {
      await mobileApi.registerPushToken({
        token,
        previous_token:
          previousToken && previousToken !== token ? previousToken : undefined,
        provider,
        platform,
      });
      await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, token);
      return {
        permission: "granted",
        token,
        registered: true,
        mode: "registered",
      };
    }

    return {
      permission: "granted",
      token,
      registered: true,
      mode: "unchanged",
    };
  } catch (error) {
    if (__DEV__) console.warn("[push] register token failed", error);
    return {
      permission: "granted",
      token,
      registered: false,
      mode: "none",
      reason: "register_failed",
    };
  }
}
