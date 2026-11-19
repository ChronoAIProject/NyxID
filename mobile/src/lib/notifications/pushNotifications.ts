import { getAuthAccountId } from "../auth/authAccount";
import { notificationDestination } from "./notificationDestination";
import {
  NYXID_APP_SCHEME,
  NYXID_TRUSTED_FRONTEND_URLS,
  NYXID_TRUSTED_API_BASE_URLS,
  NYXID_UNIVERSAL_LINK_HOST,
} from "../env";
import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import * as TaskManager from "expo-task-manager";
import { AppState, Platform } from "react-native";
import { onlineManager } from "@tanstack/react-query";
import { mobileApi } from "../api/mobileApi";
import { isApiError } from "../api/ApiError";
import { signalApprovalStateMayHaveChanged } from "./approvalRefreshSignal";
import {
  beginPushDeliveryStateHydration,
  isPushDeliverySuppressed as readPushDeliverySuppressed,
  setPushDeliverySuppressed,
  pushDeliveryStateHydrated,
} from "./pushDeliveryState";

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
const PUSH_PENDING_UNBIND_KEY = "nyxid.push.pending_unbind";
const PENDING_UNBIND_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const PUSH_NATIVE_TOKEN_TIMEOUT_MS = 15_000;
const PUSH_NATIVE_UNREGISTER_TIMEOUT_MS = 2_000;
const PUSH_LOGOUT_QUEUE_TIMEOUT_MS = 10_000;
const PUSH_LOGOUT_RECOVERY_READ_TIMEOUT_MS = 250;
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

type PushSyncHandler = (signal: PushSyncSignal | null) => void;

let pushSyncHandler: PushSyncHandler | null = null;

let isNotificationHandlerConfigured = false;
let isBackgroundTaskRegistered = false;
let isBootstrapped = false;
let pushOperationQueue: Promise<void> = Promise.resolve();
let pushOperationRevision = 0;
let lastKnownNativeToken:
  | { token: string; provider: "apns" | "fcm"; platform: "ios" | "android" }
  | undefined;

type PendingPushUnbind = {
  token: string;
  provider: "apns" | "fcm";
  platform: "ios" | "android";
  createdAt: string;
  nativeUnregisterPending?: boolean;
};

function pushAccountGuard(): () => boolean {
  const revision = pushOperationRevision;
  const account = getAuthAccountId();
  return () =>
    revision === pushOperationRevision && account === getAuthAccountId();
}

async function withPushTimeout<T>(
  operation: () => Promise<T>,
  timeoutMs: number,
): Promise<T> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation(),
      new Promise<never>((_resolve, reject) => {
        timeout = setTimeout(
          () => reject(new Error("Push operation timed out")),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}

function enqueuePushOperation<T>(operation: () => Promise<T>): Promise<T> {
  const run = pushOperationQueue.then(async () => {
    await pushDeliveryStateHydrated();
    return operation();
  });
  pushOperationQueue = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

async function loadPendingUnbind(): Promise<PendingPushUnbind | null> {
  try {
    const raw = await SecureStore.getItemAsync(PUSH_PENDING_UNBIND_KEY);
    if (!raw) {
      return null;
    }
    const value = JSON.parse(raw) as Partial<PendingPushUnbind>;
    if (
      typeof value.token !== "string" ||
      (value.platform !== "ios" && value.platform !== "android") ||
      (value.provider !== "apns" && value.provider !== "fcm") ||
      typeof value.createdAt !== "string" ||
      (value.nativeUnregisterPending !== undefined &&
        typeof value.nativeUnregisterPending !== "boolean")
    ) {
      return null;
    }
    return value as PendingPushUnbind;
  } catch {
    return null;
  }
}

export function isPushDeliverySuppressed(): boolean {
  return readPushDeliverySuppressed();
}

async function setPendingUnbind(value: PendingPushUnbind): Promise<void> {
  setPushDeliverySuppressed(true);
  await SecureStore.setItemAsync(PUSH_PENDING_UNBIND_KEY, JSON.stringify(value));
}

async function clearPendingUnbind(): Promise<void> {
  setPushDeliverySuppressed(false);
  await SecureStore.deleteItemAsync(PUSH_PENDING_UNBIND_KEY);
}

// Hydrate before notification taps are wired by the app shell. The runtime
// initializer awaits the same read before installing its listeners.
beginPushDeliveryStateHydration(
  loadPendingUnbind().then((pending) => {
    setPushDeliverySuppressed(Boolean(pending));
  }),
);

async function retryNativeUnregistration(
  pending: PendingPushUnbind,
  current: () => boolean,
): Promise<PendingPushUnbind> {
  if (!pending.nativeUnregisterPending || !current()) return pending;
  if (!(await unregisterNativePush()) || !current()) return pending;
  const updated = { ...pending, nativeUnregisterPending: false };
  await setPendingUnbind(updated);
  return updated;
}

async function unregisterNativePush(): Promise<boolean> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      if (
        Platform.OS === "web" ||
        typeof Notifications.unregisterForNotificationsAsync !== "function"
      )
        break;
      await withPushTimeout(
        () => Notifications.unregisterForNotificationsAsync(),
        PUSH_NATIVE_UNREGISTER_TIMEOUT_MS,
      );
    } catch {
      if (attempt < 2) {
        await new Promise((resolve) => setTimeout(resolve, 100 * (attempt + 1)));
        continue;
      }
      break;
    }
    return true;
  }
  if (__DEV__) console.warn("[push] native unregister unavailable");
  return false;
}

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
  url: string | null;
  title: string | null;
  body: string | null;
  challengeId: string | null;
  type: PushSyncSignalType | null;
  /** NyxBot notification pushes: the row to mark read and where to open. */
  notificationId: string | null;
  deeplink: string | null;
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

      await pushDeliveryStateHydrated();
      if (readPushDeliverySuppressed()) {
        return SUPPRESS_OS_NOTIFICATION;
      }

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
  if (readPushDeliverySuppressed() || AppState.currentState !== "active") return;

  const title = asNonEmptyString(content.title);
  const body = asNonEmptyString(content.body);
  if (!title && !body) return;

  // The user already has this exact approval open. Announcing it on top of
  // the screen they are acting on is pure noise.
  if (signal && activeChallengeId && signal.challengeId === activeChallengeId) {
    return;
  }

  const data = content.data as
    | { notification_id?: unknown; deeplink?: unknown; deep_link?: unknown }
    | null
    | undefined;
  const destination = notificationDestination(content.data, {
    scheme: NYXID_APP_SCHEME,
    trustedUrls: [
      ...NYXID_TRUSTED_FRONTEND_URLS,
      ...NYXID_TRUSTED_API_BASE_URLS,
      ...(NYXID_UNIVERSAL_LINK_HOST
        ? [`https://${NYXID_UNIVERSAL_LINK_HOST}`]
        : []),
    ],
  });
  foregroundNotificationHandler?.({
    url: destination ?? `${NYXID_APP_SCHEME}://activity`,
    title,
    body,
    challengeId: signal?.challengeId ?? null,
    type: signal?.type ?? null,
    notificationId: destination ? asNonEmptyString(data?.notification_id) : null,
    deeplink:
      asNonEmptyString(data?.deeplink) ?? asNonEmptyString(data?.deep_link),
  });
}

/**
 * Must be called at module-load time (index.ts), BEFORE registerRootComponent.
 * Sets up notification handler + background task + Android channels so that
 * FCM messages arriving when the app is killed still produce visible alerts.
 * This pre-gate setup only configures native delivery and persists signals;
 * API registration and refresh start in App after DomainGate selects domains.
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
    enableVibrate: true,
    showBadge: true,
  });

  await Notifications.setNotificationChannelAsync("approvals", {
    name: "Approvals",
    description: "Approval requests and decisions",
    importance: Notifications.AndroidImportance.MAX,
    vibrationPattern: [0, 200, 200, 200],
    lightColor: "#9775fa",
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
  if (pushSyncHandler && AppState.currentState === "active") {
    pushSyncHandler(signal);
    return;
  }
  await persistPendingPushSyncSignal(signal);
}

function emitGenericApprovalRefreshHint(): void {
  if (pushSyncHandler && AppState.currentState === "active") {
    // `null` means the notification could not be attributed to one challenge;
    // App still invalidates both list families before notifying the focused UI.
    pushSyncHandler(null);
    return;
  }
  signalApprovalStateMayHaveChanged();
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
      // Expo's notification task is the iOS delivery path for data-only
      // content-available pushes even while the app is foregrounded. Deliver
      // into the mounted runtime when possible; persist only for background or
      // headless execution so resume/mount consumes it durably.
      await emitOrPersistPushSyncSignal(signal);
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
  let appStateSubscription: { remove: () => void } | null = null;
  let networkSubscription: (() => void) | null = null;

  try {
    await pushDeliveryStateHydrated();
    configureNotificationHandler();

    foregroundSubscription = Notifications.addNotificationReceivedListener(
      (notification) => {
        const content = notification.request.content;
        const signal = parsePushSyncSignalFromData(content.data, "foreground");

        if (signal) {
          void emitOrPersistPushSyncSignal(signal);
        } else {
          const type =
            content.data && typeof content.data === "object"
              ? asNonEmptyString((content.data as { type?: unknown }).type)
              : null;
          const hasVisibleCopy =
            asNonEmptyString(content.title) !== null ||
            asNonEmptyString(content.body) !== null;

          // Approval decision/expiry pushes are normally parsed above, but a
          // rolling backend or provider transform can strip an id or introduce
          // a new approval_* type. Any unattributed visible toast is also a
          // conservative refresh hint: it is better to make one throttled list
          // request than show a toast beside approval state we cannot prove is
          // current. Known non-approval silent pushes remain silent here.
          if (hasVisibleCopy || type?.startsWith("approval_")) {
            emitGenericApprovalRefreshHint();
          }
        }

        // Runs for non-approval pushes too -- those may have no parsed sync
        // signal but still deserve the in-app toast that replaced their OS one.
        presentForegroundNotification(content, signal);
      }
    );

    tokenSubscription = safeAddPushTokenListener((devicePushToken) => {
      if (!getAuthAccountId()) return;
      const token = normalizeDeviceToken(devicePushToken);
      if (token) void syncTokenWithBackend(token, false);
    });
    appStateSubscription = AppState.addEventListener("change", (state) => {
      if (state === "active" && readPushDeliverySuppressed())
        void activatePushAfterLogin({ forceRegister: true });
    });
    networkSubscription = onlineManager.subscribe(() => {
      if (onlineManager.isOnline() && readPushDeliverySuppressed())
        void activatePushAfterLogin({ forceRegister: true });
    });
    void enqueuePushOperation(async () => {
      const current = pushAccountGuard();
      const pending = await loadPendingUnbind();
      if (pending) await retryNativeUnregistration(pending, current);
    }).catch((error) => {
      if (__DEV__) console.warn("[push] native unregister retry failed", error);
    });
    void Promise.all([
      ensureAndroidChannels(),
      ensureBackgroundTaskRegistered(),
    ]).catch((error) => {
      if (__DEV__) console.warn("[push] initialize runtime failed", error);
    });
  } catch (error) {
    if (__DEV__) console.warn("[push] initialize runtime failed", error);
  }

  return () => {
    foregroundSubscription?.remove();
    tokenSubscription?.remove();
    appStateSubscription?.remove();
    networkSubscription?.();
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

type PushLogoutCapture = { accessToken?: string; registration: PendingPushUnbind | null };

export async function capturePushLogout(accessToken?: string): Promise<PushLogoutCapture> {
  const token = lastKnownNativeToken?.token ?? await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY);
  const platform = lastKnownNativeToken?.platform ?? resolvePlatform();
  const provider = lastKnownNativeToken?.provider ?? (platform === "ios" || platform === "android" ? resolveProvider(platform) : null);
  return { accessToken, registration: token && provider && (platform === "ios" || platform === "android")
    ? { token, platform, provider, createdAt: new Date().toISOString(), nativeUnregisterPending: true }
    : await loadPendingUnbind() };
}

export async function deactivatePushOnLogout(captured?: PushLogoutCapture): Promise<boolean> {
  pushOperationRevision++;
  const logoutGuard = pushAccountGuard();
  if (captured) {
    // Fence a register already in flight. Persist recovery before any network
    // wait so a cold start suppresses stale-account notification delivery.
    if (captured.registration) {
      try { await setPendingUnbind(captured.registration); } catch (error) {
        // Even a recovery-record fault must try to invalidate native delivery.
        if (logoutGuard()) await unregisterNativePush();
        throw error;
      }
    }
  }
  let queueTimedOut = false;
  const queuedUnbind = enqueuePushOperation(async () => {
    if (queueTimedOut) return false;
    const current = logoutGuard;
    if (!current()) return false;
    const storedToken = await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY);
    // A first registration may fail after the server accepted it. Keep the
    // token in memory until logout reaches this queue so the authenticated
    // DELETE still has a chance to detach that uncertain registration.
    const token = captured?.registration?.token ?? lastKnownNativeToken?.token ?? storedToken;
    const platform = captured?.registration?.platform ?? lastKnownNativeToken?.platform ?? resolvePlatform();
    const pending = await loadPendingUnbind();
    if (!current()) return false;
    const provider =
      captured?.registration?.provider ?? lastKnownNativeToken?.provider ??
      (platform === "ios" || platform === "android"
        ? resolveProvider(platform)
        : null);
    if (!token || !provider || (platform !== "ios" && platform !== "android")) {
      if (!pending) await clearPendingUnbind();
      await clearLocalPushRegistrationState();
      lastKnownNativeToken = undefined;
      return !pending;
    }
    try {
      if (captured?.accessToken) await mobileApi.unregisterPushToken({ token, provider, platform }, captured.accessToken);
      else await mobileApi.unregisterPushToken({ token, provider, platform });
      if (!current()) return false;
      if (!pending || pending.token === token) await clearPendingUnbind();
      await clearLocalPushRegistrationState();
      lastKnownNativeToken = undefined;
      return true;
    } catch {
      if (!current()) return false;
      return failPushUnbind(current);
    }
  });
  let unbound: boolean;
  try {
    unbound = await withPushTimeout(
      () => queuedUnbind,
      PUSH_LOGOUT_QUEUE_TIMEOUT_MS,
    );
  } catch {
    queueTimedOut = true;
    // The start-of-logout revision already fenced earlier work. Recovery must
    // keep that owner: a replacement account may already be queued behind us.
    try {
      unbound = await failPushUnbind(logoutGuard);
    } catch {
      // Local cleanup failures must not prevent clearing the auth session.
      if (__DEV__) console.warn("[push] logout recovery could not be persisted");
      unbound = false;
    }
  }
  // Each cleanup is independent: a badge failure must not retain tray entries
  // or a cached registration that could skip the next account's bind.
  if (logoutGuard()) await Promise.allSettled([
    withPushTimeout(
      () => Notifications.dismissAllNotificationsAsync(),
      PUSH_NATIVE_UNREGISTER_TIMEOUT_MS,
    ),
    withPushTimeout(
      () => Notifications.setBadgeCountAsync(0),
      PUSH_NATIVE_UNREGISTER_TIMEOUT_MS,
    ),
    withPushTimeout(
      () => Notifications.clearLastNotificationResponseAsync(),
      PUSH_NATIVE_UNREGISTER_TIMEOUT_MS,
    ),
  ]);
  return unbound;
}

async function failPushUnbind(
  current = pushAccountGuard(),
): Promise<false> {
  let storedToken: string | null = null;
  let pending: PendingPushUnbind | null = null;
  try {
    [storedToken, pending] = await withPushTimeout(
      () => Promise.all([SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY), loadPendingUnbind()]),
      PUSH_LOGOUT_RECOVERY_READ_TIMEOUT_MS,
    );
  } catch (error) {
    // Missing capture or unreadable storage must still reach native invalidation.
    if (__DEV__) console.warn("[push] logout recovery read unavailable", error);
  }
  if (!current()) return false;
  const token = lastKnownNativeToken?.token ?? storedToken ?? pending?.token;
  const platform = lastKnownNativeToken?.platform ?? resolvePlatform();
  const provider =
    lastKnownNativeToken?.provider ??
    (platform === "ios" || platform === "android"
      ? resolveProvider(platform)
      : null);
  if (token && provider && (platform === "ios" || platform === "android")) {
    const failedUnbind: PendingPushUnbind = {
      token,
      provider,
      platform,
      createdAt: new Date().toISOString(),
      nativeUnregisterPending: true,
    };
    await setPendingUnbind(failedUnbind);
    await retryNativeUnregistration(failedUnbind, current);
  } else {
    await unregisterNativePush();
  }
  if (!current()) return false;
  await clearLocalPushRegistrationState();
  lastKnownNativeToken = undefined;
  return false;
}

export async function activatePushAfterLogin(
  options: PushActivateOptions = {}
): Promise<PushActivateResult> {
  const forceRegister = options.forceRegister === true;
  const current = pushAccountGuard();
  return enqueuePushOperation(async () => {
    if (!current()) return signedOutPushResult();
    const pending = await loadPendingUnbind();
    if (pending) return rebindPendingToken(pending, current);
    if (!getAuthAccountId()) return signedOutPushResult();
    return activatePushAfterLoginInQueue(forceRegister, current);
  });
}

async function syncTokenWithBackend(
  token: string,
  forceRegister: boolean
): Promise<PushActivateResult> {
  const current = pushAccountGuard();
  const platform = resolvePlatform();
  if (platform === "ios" || platform === "android") {
    lastKnownNativeToken = {
      token,
      provider: resolveProvider(platform),
      platform,
    };
  }
  return enqueuePushOperation(async () => {
    if (!current()) return signedOutPushResult();
    const pending = await loadPendingUnbind();
    if (pending) return rebindPendingToken(pending, current);
    return syncTokenWithBackendInQueue(token, forceRegister, current);
  });
}

async function activatePushAfterLoginInQueue(
  forceRegister: boolean,
  current: () => boolean,
  previousTokenOverride?: string,
): Promise<PushActivateResult> {
  const permission = await ensureNotificationPermission();
  if (!current()) return signedOutPushResult();
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
    token = normalizeDeviceToken(
      await withPushTimeout(
        () => Notifications.getDevicePushTokenAsync(),
        PUSH_NATIVE_TOKEN_TIMEOUT_MS,
      ),
    );
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
  if (!current()) return signedOutPushResult();
  if (!token) {
    return {
      permission,
      token: null,
      registered: false,
      mode: "none",
      reason: "token_unavailable",
    };
  }
  return syncTokenWithBackendInQueue(
    token,
    forceRegister,
    current,
    previousTokenOverride,
  );
}

async function rebindPendingToken(
  pending: PendingPushUnbind,
  current: () => boolean,
): Promise<PushActivateResult> {
  if (!current()) return signedOutPushResult();
  const createdAt = Date.parse(pending.createdAt);
  if (
    !Number.isFinite(createdAt) ||
    Date.now() - createdAt > PENDING_UNBIND_MAX_AGE_MS
  ) {
    await clearPendingUnbind();
    if (!getAuthAccountId()) return signedOutPushResult();
    return activatePushAfterLoginInQueue(true, current);
  }
  await retryNativeUnregistration(pending, current);
  if (!current()) return signedOutPushResult();
  const accountId = getAuthAccountId();
  if (!accountId) {
    return {
      permission: "granted",
      token: pending.token,
      registered: false,
      mode: "none",
      reason: "register_failed",
    };
  }
  try {
    await mobileApi.registerPushToken({
      token: pending.token,
      provider: pending.provider,
      platform: pending.platform,
    });
  } catch (error) {
    if (!current()) return signedOutPushResult();
    if (
      isApiError(error) &&
      error.statusCode >= 400 &&
      error.statusCode < 500 &&
      ![401, 408, 429].includes(error.statusCode)
    ) {
      // Register detaches other accounts before its device-cap check. Invalid
      // token formats fail before detach but cannot receive pushes anyway.
      await clearPendingUnbind();
      return activatePushAfterLoginInQueue(true, current);
    }
    if (__DEV__) console.warn("[push] pending token rebind failed", error);
    return {
      permission: "granted",
      token: pending.token,
      registered: false,
      mode: "none",
      reason: "register_failed",
    };
  }

  if (!current()) return signedOutPushResult();
  await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, pending.token);
  await clearPendingUnbind();
  return activatePushAfterLoginInQueue(false, current, pending.token);
}

function signedOutPushResult(): PushActivateResult {
  return {
    permission: "granted",
    token: null,
    registered: false,
    mode: "none",
    reason: "register_failed",
  };
}

async function syncTokenWithBackendInQueue(
  token: string,
  forceRegister: boolean,
  current: () => boolean,
  previousTokenOverride?: string,
): Promise<PushActivateResult> {
  const accountId = getAuthAccountId();
  const previousToken =
    previousTokenOverride ??
    (await SecureStore.getItemAsync(PUSH_TOKEN_STORE_KEY));
  const platform = resolvePlatform();
  if (!accountId || !current())
    return {
      permission: "granted" as const,
      token,
      registered: false,
      mode: "none" as const,
      reason: "register_failed" as const,
    };
  if (platform !== "ios" && platform !== "android") {
    return {
      permission: "granted" as const,
      token,
      registered: false,
      mode: "none" as const,
      reason: "register_failed" as const,
    };
  }
  const provider = resolveProvider(platform);
  lastKnownNativeToken = { token, provider, platform };

  try {
    if (previousToken && previousToken !== token) {
      await mobileApi.rotatePushToken({
        token,
        previous_token: previousToken,
        provider,
        platform,
      });
      if (!current()) return signedOutPushResult();
      await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, token);
      return {
        permission: "granted" as const,
        token,
        registered: true,
        mode: "rotated" as const,
      };
    }
    if (!previousToken || forceRegister) {
      await mobileApi.registerPushToken({ token, provider, platform });
      if (!current()) return signedOutPushResult();
      await SecureStore.setItemAsync(PUSH_TOKEN_STORE_KEY, token);
      return {
        permission: "granted" as const,
        token,
        registered: true,
        mode: "registered" as const,
      };
    }
    return {
      permission: "granted" as const,
      token,
      registered: true,
      mode: "unchanged" as const,
    };
  } catch (error) {
    if (__DEV__) console.warn("[push] register token failed", error);
    return {
      permission: "granted" as const,
      token,
      registered: false,
      mode: "none" as const,
      reason: "register_failed" as const,
    };
  }
}
