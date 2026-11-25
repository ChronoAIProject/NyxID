import { useDiagnosticsNotice } from "../lib/crashReporting/useDiagnosticsNotice";
import { usePerformanceLifecycle } from "../lib/performance/useNativePerformance";
import { newAssistantChatParams } from "./assistantChatParams";
import { workspaceCornerSensor } from "../components/workspace/workspaceLayout";
import { useAppLanguage } from "../i18n/LanguageProvider";
import { translateApp } from "../i18n/appTranslation";
import { LanguageProvider } from "../i18n/LanguageProvider";
import "react-native-gesture-handler";
import { useForegroundNotificationToast } from "./useForegroundNotificationToast";
import { useNotificationNavigation } from "./useNotificationNavigation";
import {
  WorkspaceAccessibilityContext,
  workspaceAccessibilityState,
  type WorkspaceAccessibility,
} from "../lib/accessibility/workspaceAccessibility";
import {
  type WorkspaceContentTransitionHandle,
  WorkspacePushShell,
} from "../components/workspace/WorkspacePushShell";
import { workspaceFocus } from "../lib/accessibility/workspaceFocus";
import { getAuthAccountId, subscribeAuthAccount } from "../lib/auth/authAccount";
import { getAuthEpoch } from "../lib/auth/authEpoch";
import { StatusBar } from "expo-status-bar";
import * as Notifications from "expo-notifications";
import * as SplashScreen from "expo-splash-screen";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

// Keep the native splash visible until we've loaded fonts. The
// expo-splash-screen plugin disables Expo's default auto-hide, so the
// app must explicitly hide the splash itself (see effect below).
SplashScreen.preventAutoHideAsync().catch(() => {
  /* no-op — preventAutoHideAsync rejects if the splash has already
     been hidden, e.g. during fast refresh. Safe to ignore. */
});
import {
  ActivityIndicator,
  AppState,
  findNodeHandle,
  Keyboard,
  StyleSheet,
  Text,
  useWindowDimensions,
  Platform,
  View,
} from "react-native";
import {
  SafeAreaProvider,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import {
  onlineManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import NetInfo from "@react-native-community/netinfo";
import {
  NavigationContainer,
  type NavigationAction,
  NavigationState,
  PartialState,
  useNavigationContainerRef,
} from "@react-navigation/native";
import { useFonts } from "expo-font";
import {
  Manrope_400Regular,
  Manrope_500Medium,
  Manrope_600SemiBold,
  Manrope_700Bold,
} from "@expo-google-fonts/manrope";
import {
  SpaceGrotesk_500Medium,
  SpaceGrotesk_600SemiBold,
  SpaceGrotesk_700Bold,
} from "@expo-google-fonts/space-grotesk";
import { JetBrainsMono_400Regular } from "@expo-google-fonts/jetbrains-mono";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { AppNavigator, RootStackParamList } from "./AppNavigator";
import { appLinking, notificationLinkConfig } from "./linking";
import {
  isWorkspaceSidebarRoot,
  openScannerSheetAction,
  openWorkspaceSheetAction,
  workspaceDestinationMatches,
  workspaceContentIdentity,
  workspaceNavigateAction,
} from "./navigationHierarchy";
import {
  WorkspaceSidebarToggleContext,
  type WorkspaceNavigationGuardChange,
  type WorkspaceEdgeBackChange,
} from "./WorkspaceHeader";
import { useSidebarOpen } from "../components/workspace/workspaceSidebarPreference";
import {
  consumePendingPushSyncSignal,
  initializeNotificationRuntime,
  PushSyncSignal,
  setPushSyncHandler,
} from "../lib/notifications/pushNotifications";
import { ToastOverlay, type ToastState } from "../components/ToastOverlay";
import { signalApprovalStateMayHaveChanged } from "../lib/notifications/approvalRefreshSignal";
import {
  AuthSessionProvider,
  useAuthSession,
} from "../features/auth/AuthSessionContext";
import { MobileConsentProvider } from "../lib/consent";
import { capture } from "../lib/telemetry";
import type { PushType } from "../lib/telemetry-schema";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import {
  WorkspaceDrawer,
  type WorkspaceRoute,
} from "../components/workspace/WorkspaceDrawer";
import { WorkspaceQuickRail } from "../components/workspace/WorkspaceQuickRail";
import type { QuickAccessItem } from "../lib/api/quickAccessSchema";
import { notificationsApi } from "../lib/api/notificationsApi";
import { createNotificationPushController } from "../lib/notifications/notificationPushController";
import { ChatLayoutProvider } from "../features/display/ChatLayoutContext";
import { workspaceSidebarLayout } from "../components/workspace/workspacePushState";
import { ThemeProvider, useTheme } from "../theme/ThemeContext";
import {
  activeWorkspaceChatTarget,
  workspaceChatNavigationAction,
  type WorkspaceChatTarget,
} from "./workspaceChatNavigation";

// Wire TanStack Query's online state to NetInfo so queries pause when offline
// and auto-refetch when connectivity returns.
onlineManager.setEventListener((setOnline) => {
  return NetInfo.addEventListener((state) => {
    setOnline(!!state.isConnected);
  });
});

const queryClient = new QueryClient();

function refreshQueryCacheFromPushSignal(signal: PushSyncSignal | null) {
  // Notification callbacks can run while the app is inactive. Mark cached
  // approval data stale here, but leave network work to the focused Activity
  // subscriber (foreground) or the existing active-query resume sweep.
  void queryClient.invalidateQueries({
    queryKey: ["challenges"],
    refetchType: "none",
  });
  void queryClient.invalidateQueries({
    queryKey: ["approvals"],
    refetchType: "none",
  });
  if (signal) {
    void queryClient.invalidateQueries({
      queryKey: ["challenge", signal.challengeId],
      // The detail query owns this key and historically refreshed live. Keep
      // that behavior in the foreground, while the focused signal subscriber
      // handles background/offline catch-up without starting network work.
      refetchType:
        AppState.currentState === "active" && onlineManager.isOnline()
          ? "active"
          : "none",
    });
  }
  signalApprovalStateMayHaveChanged();
}

function getActiveRouteName(
  state: NavigationState | PartialState<NavigationState> | undefined,
): string | undefined {
  if (!state || state.routes.length === 0) return undefined;

  const index = state.index ?? 0;
  const route = state.routes[index];
  if (!route) return undefined;
  const nested = route.state as
    | NavigationState
    | PartialState<NavigationState>
    | undefined;
  if (nested) {
    return getActiveRouteName(nested);
  }

  return route.name;
}

export default function App() {
  return (
    <LanguageProvider>
      <AppContent />
    </LanguageProvider>
  );
}

function AppContent() {
  useAppLanguage();
  usePerformanceLifecycle();
  const navigationRef = useNavigationContainerRef<RootStackParamList>();
  const [currentRouteName, setCurrentRouteName] = useState<string | undefined>(
    undefined,
  );
  const navigationGuardsRef = useRef(new Set<symbol>());
  const criticalNavigationGuards = useRef(new Set<symbol>());
  const [navigationGuardRevision, setNavigationGuardRevision] = useState(0);
  const lastAppStateRef = useRef(AppState.currentState);

  const [pushController] = useState(() =>
    createNotificationPushController({
      api: notificationsApi,
      queryClient,
      isForeground: () => AppState.currentState === "active",
      isOnline: () => onlineManager.isOnline(),
    }),
  );
  const { open: openNotification, flush: flushPendingNotificationNavigation } =
    useNotificationNavigation(
      navigationRef,
      navigationGuardsRef,
      pushController.navigated,
    );

  const onNavigationGuardChange = useCallback<WorkspaceNavigationGuardChange>(
    (id, blocked, critical = false) => {
      const guards = navigationGuardsRef.current;
      if (blocked && critical) criticalNavigationGuards.current.add(id);
      else criticalNavigationGuards.current.delete(id);
      if (blocked) {
        guards.add(id);
      } else if (!guards.delete(id)) {
        return;
      }
      setNavigationGuardRevision((value) => value + 1);
    },
    [],
  );

  const isNavigationBlocked = useCallback(
    () => criticalNavigationGuards.current.size > 0,
    [],
  );

  // Guard cleanup schedules a normal effect; it never dispatches during the
  // header's layout effect or re-enters an in-progress navigation transition.
  useEffect(() => {
    flushPendingNotificationNavigation();
  }, [navigationGuardRevision, flushPendingNotificationNavigation]);

  // Tapping the in-app toast resolves to exactly the same navigation as
  // tapping an OS notification, including the "Activity route not mounted
  // yet" deferral -- the pending id survives until the navigator is ready
  // and `onStateChange` flushes it.
  const openChallengeFromNotification = useCallback(
    (challengeId: string) => {
      openNotification(
        `${notificationLinkConfig.scheme}://activity?${new URLSearchParams({ challengeId })}`,
      );
    },
    [openNotification],
  );

  const [fontsLoaded] = useFonts({
    Manrope_400Regular,
    Manrope_500Medium,
    Manrope_600SemiBold,
    Manrope_700Bold,
    SpaceGrotesk_500Medium,
    SpaceGrotesk_600SemiBold,
    SpaceGrotesk_700Bold,
    JetBrainsMono_400Regular,
  });
  const [fontLoadTimeout, setFontLoadTimeout] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => setFontLoadTimeout(true), 8000);
    return () => clearTimeout(t);
  }, []);

  // Hide the native splash once we have enough state to render the UI.
  // expo-splash-screen plugin disables the auto-hide, so without this
  // call the splash never goes away.
  const ready = fontsLoaded || fontLoadTimeout;
  useEffect(() => {
    if (ready) {
      SplashScreen.hideAsync().catch(() => {
        /* already hidden — fine */
      });
    }
  }, [ready]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void initializeNotificationRuntime()
      .then((unsubscribe) => {
        if (disposed) {
          unsubscribe();
          return;
        }
        cleanup = unsubscribe;
      })
      .catch((error) => {
        if (__DEV__)
          console.warn("[push] notification runtime bootstrap failed", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  // Emit `mobile.push_received` — narrow categorization only. We never
  // read the notification body; only infer the `type` from the structured
  // data field (set by the backend push payload) and snapshot the
  // current AppState to classify foreground vs background delivery.
  useEffect(() => {
    const classifyPushType = (data: unknown): PushType => {
      if (!data || typeof data !== "object") return "other";
      const typeRaw = (data as { type?: unknown }).type;
      if (typeof typeRaw === "string" && typeRaw === "approval_request") {
        return "approval_request";
      }
      // Backend payloads may omit `type` on the approval path but still
      // ship a `challenge_id`/`request_id`, which uniquely identifies
      // an approval request. Anything else (decision ack, expiry)
      // is lumped into "other".
      if (!typeRaw) {
        const challengeId =
          (data as { challenge_id?: unknown }).challenge_id ??
          (data as { challengeId?: unknown }).challengeId ??
          (data as { request_id?: unknown }).request_id;
        if (typeof challengeId === "string" && challengeId.length > 0) {
          return "approval_request";
        }
      }
      return "other";
    };

    // NOTE: `addNotificationReceivedListener` only fires while the JS
    // runtime is active -- i.e., the app is foregrounded or briefly
    // active in the background. Pushes delivered while the app is
    // terminated, or shown by the OS without waking JS, are NOT observed
    // here. This listener therefore reliably measures ONLY
    // `app_state: "foreground"` receipts. Background/quit delivery
    // observability would require a native notification-service
    // extension on iOS and a foreground service on Android, which is
    // out of scope for this sweep. See docs/TELEMETRY.md §5.3. The
    // `app_state` prop is still populated from `AppState.currentState`
    // for honest emission when the listener does happen to catch a
    // backgrounded-but-live runtime; under normal conditions it reads
    // "foreground".
    const subscription = Notifications.addNotificationReceivedListener(
      (notification) => {
        try {
          const pushType = classifyPushType(notification.request.content.data);
          const appState =
            AppState.currentState === "active" ? "foreground" : "background";
          capture({
            name: "mobile.push_received",
            props: { type: pushType, app_state: appState },
          });
        } catch {
          // never break push pipeline on telemetry failure
        }
        pushController.received(notification.request.content.data);
      },
    );

    return () => {
      subscription.remove();
    };
  }, [pushController]);

  const { toast: pushToast, toastKey: pushToastKey } =
    useForegroundNotificationToast(
      openChallengeFromNotification,
      openNotification,
    );

  useEffect(() => {
    const onPushSyncSignal = (signal: PushSyncSignal | null) => {
      refreshQueryCacheFromPushSignal(signal);
    };

    const disposePushSyncHandler = setPushSyncHandler(onPushSyncSignal);

    const consumePendingSignal = async () => {
      const pendingSignal = await consumePendingPushSyncSignal();
      if (pendingSignal) {
        refreshQueryCacheFromPushSignal(pendingSignal);
      }
    };

    void consumePendingSignal();

    const appStateSubscription = AppState.addEventListener(
      "change",
      (nextState) => {
        const previousState = lastAppStateRef.current;
        lastAppStateRef.current = nextState;

        const resumedFromBackground =
          (previousState === "background" || previousState === "inactive") &&
          nextState === "active";

        if (!resumedFromBackground) return;

        void consumePendingSignal();
        void queryClient.refetchQueries({ type: "active" });
      },
    );

    return () => {
      disposePushSyncHandler();
      appStateSubscription.remove();
    };
  }, []);

  const canShowApp = fontsLoaded || fontLoadTimeout;
  if (!canShowApp) {
    return (
      <View style={appLoadingStyles.container}>
        <StatusBar style="light" />
        <ActivityIndicator size="large" color="#9775fa" />
        <Text style={appLoadingStyles.text}>{translateApp("Loading...")}</Text>
      </View>
    );
  }

  return (
    <AppErrorBoundary>
      <GestureHandlerRootView style={appRootStyles.fill}>
        <SafeAreaProvider>
          <QueryClientProvider client={queryClient}>
            <ThemeProvider>
              <ChatLayoutProvider>
                <MobileConsentProvider>
                  <AuthSessionProvider>
                    <ThemedAppShell
                      navigationRef={navigationRef}
                      currentRouteName={currentRouteName}
                      setCurrentRouteName={setCurrentRouteName}
                      flushPendingNotificationNavigation={
                        flushPendingNotificationNavigation
                      }
                      onNavigationGuardChange={onNavigationGuardChange}
                      navigationBlocked={isNavigationBlocked()}
                      isNavigationBlocked={isNavigationBlocked}
                      pushToast={pushToast}
                      pushToastKey={pushToastKey}
                    />
                  </AuthSessionProvider>
                </MobileConsentProvider>
              </ChatLayoutProvider>
            </ThemeProvider>
          </QueryClientProvider>
        </SafeAreaProvider>
      </GestureHandlerRootView>
    </AppErrorBoundary>
  );
}

function ThemedAppShell({
  navigationRef,
  currentRouteName,
  setCurrentRouteName,
  flushPendingNotificationNavigation,
  onNavigationGuardChange,
  navigationBlocked,
  isNavigationBlocked,
  pushToast,
  pushToastKey,
}: {
  navigationRef: ReturnType<
    typeof useNavigationContainerRef<RootStackParamList>
  >;
  currentRouteName: string | undefined;
  setCurrentRouteName: (name: string | undefined) => void;
  flushPendingNotificationNavigation: () => void;
  onNavigationGuardChange: WorkspaceNavigationGuardChange;
  navigationBlocked: boolean;
  isNavigationBlocked: () => boolean;
  pushToast: ToastState | null;
  pushToastKey: number;
}) {
  useAppLanguage();
  const { mode } = useTheme();
  const insets = useSafeAreaInsets();
  const { isAuthenticated } = useAuthSession();
  const sessionEpoch = getAuthEpoch();
  const { width, height } = useWindowDimensions();
  const cornerWindow = workspaceCornerSensor({
    platform: Platform.OS,
    width,
    height,
    rightInset: insets.right,
  });
  const persistentSidebar =
    isAuthenticated && workspaceSidebarLayout(width).persistent;
  const reviewDiagnostics = useCallback(() => {
    if (navigationRef.isReady()) {
      navigationRef.navigate("WorkspaceSheet", { screen: "PrivacySettings" });
    }
  }, [navigationRef]);
  useDiagnosticsNotice(isAuthenticated, reviewDiagnostics);
  // Wide windows: the persistent sidebar is closed by default (content full
  // width); Menu opens or hides it and the choice is saved per device,
  // independent of the Duo's full screen. (`sidebarOpen` below is the
  // overlay drawer's accessibility state.)
  const [persistentSidebarOpen, setPersistentSidebarOpen, sidebarRestored] =
    useSidebarOpen();
  const sidebarCollapsed = cornerWindow && !persistentSidebarOpen;
  const authenticatedNow = useRef(isAuthenticated);
  authenticatedNow.current = isAuthenticated && getAuthAccountId() !== null;
  const edgeBacks = useRef(new Map<symbol, () => void>());
  const [edgeBackRevision, setEdgeBackRevision] = useState(0);
  const onEdgeBackChange = useCallback<WorkspaceEdgeBackChange>((id, back) => {
    if (back) edgeBacks.current.set(id, back);
    else edgeBacks.current.delete(id);
    setEdgeBackRevision((value) => value + 1);
  }, []);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [drawerPresented, setDrawerPresented] = useState(false);
  // Read by navigation callbacks that may come from an older render.
  const drawerPresentedNow = useRef(drawerPresented);
  drawerPresentedNow.current = drawerPresented;
  const sidebarFocus = useRef<View>(null);
  const drawerOrigin =
    useRef<ReturnType<typeof workspaceFocus.capture>>(undefined);
  const pendingMenuFocus =
    useRef<ReturnType<typeof workspaceFocus.capture>>(undefined);
  const pendingDrawerNavigation = useRef<NavigationAction | null>(null);
  const contentTransition = useRef<WorkspaceContentTransitionHandle | null>(
    null,
  );
  const didNavigate = useRef(false);
  const awaitingPaint = useRef<string | null>(null);
  const paintedPage = useRef<string | null>(null);
  const selectionWatchdog = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const revealContent = useCallback(() => {
    clearTimeout(selectionWatchdog.current);
    selectionWatchdog.current = undefined;
    awaitingPaint.current = null;
    contentTransition.current?.reveal();
  }, []);
  const cancelSelection = useCallback(() => {
    // Fence an outstanding fade callback before making the current root visible.
    pendingDrawerNavigation.current = null;
    revealContent();
  }, [revealContent]);
  useEffect(
    () => () => {
      clearTimeout(selectionWatchdog.current);
      pendingDrawerNavigation.current = null;
    },
    [],
  );
  const priorSheet = useRef<{ open: boolean; content: string; tracked: boolean; }>({
    open: false,
    content: "",
    tracked: false,
  });
  const [currentChat, setCurrentChat] = useState<WorkspaceChatTarget>();
  const [accessibility, setAccessibility] = useState<WorkspaceAccessibility>({
    sheetOpen: false,
  });
  const sidebarOpen = !persistentSidebar && drawerPresented;
  const workspaceAccessibility = useMemo(
    () => ({ ...accessibility, sidebarOpen }),
    [accessibility, sidebarOpen],
  );
  const trackCurrentRoute = () => {
    const state = navigationRef.getRootState();
    const next = workspaceAccessibilityState(state);
    const content = workspaceContentIdentity(state);
    if (
      authenticatedNow.current &&
      !persistentSidebar &&
      ((pendingDrawerNavigation.current &&
        !didNavigate.current &&
        workspaceDestinationMatches(state, pendingDrawerNavigation.current)) ||
        // Only a presented sidebar waits to close after a sheet's destination;
        // with none, nothing would ever dismiss and clear this selection.
        (drawerPresentedNow.current &&
          priorSheet.current.tracked &&
          priorSheet.current.open &&
          !next.sheetOpen &&
          content !== priorSheet.current.content))
    ) {
      didNavigate.current = true;
      awaitingPaint.current = content;
      if (paintedPage.current === content) {
        revealContent();
        setDrawerOpen(false);
      }
    }
    priorSheet.current = {
      open: next.sheetOpen,
      content,
      tracked: authenticatedNow.current,
    };
    workspaceFocus.context(next.primaryKey, next.sheetOpen);
    setAccessibility(next);
    setCurrentRouteName(getActiveRouteName(state));
    setCurrentChat(
      authenticatedNow.current
        ? activeWorkspaceChatTarget(navigationRef.getCurrentRoute())
        : undefined,
    );
  };
  const resetWorkspaceSession = useCallback(() => {
    authenticatedNow.current = isAuthenticated && getAuthAccountId() !== null;
    cancelSelection();
    paintedPage.current = null;
    didNavigate.current = false;
    const state = navigationRef.isReady()
      ? navigationRef.getRootState()
      : undefined;
    const next = state
      ? workspaceAccessibilityState(state)
      : { sheetOpen: false };
    priorSheet.current = {
      open: next.sheetOpen,
      content: workspaceContentIdentity(state),
      // A retained sheet belongs to the old session until this one tracks it.
      tracked: false,
    };
    drawerOrigin.current = undefined;
    pendingMenuFocus.current = undefined;
    setDrawerOpen(false);
    setDrawerPresented(false);
    setCurrentChat(undefined);
    setCurrentRouteName(
      authenticatedNow.current ? getActiveRouteName(state) : undefined,
    );
    setAccessibility(next);
    workspaceFocus.context(next.primaryKey, next.sheetOpen);
  }, [
    cancelSelection,
    navigationRef,
    setCurrentRouteName,
    isAuthenticated
  ]);
  useEffect(() => {
    resetWorkspaceSession();
    // Account admission also covers a direct account switch or batched
    // sign-out/sign-in that never renders isAuthenticated=false.
    return subscribeAuthAccount(resetWorkspaceSession);
  }, [isAuthenticated, resetWorkspaceSession]);
  useEffect(() => {
    // Resizing changes the presentation, never the navigator or its drafts.
    // Fence a pending compact animation before exposing both tablet columns.
    cancelSelection();
    didNavigate.current = false;
    drawerOrigin.current = undefined;
    pendingMenuFocus.current = undefined;
    setDrawerOpen(false);
    setDrawerPresented(false);
  }, [persistentSidebar, cancelSelection]);
  useEffect(
    () => () => workspaceFocus.releaseDrawer("persistent-workspace-sidebar"),
    [persistentSidebar, accessibility.sheetOpen],
  );
  useEffect(() => {
    if (drawerPresented) return;
    const origin = pendingMenuFocus.current;
    pendingMenuFocus.current = undefined;
    if (isAuthenticated) workspaceFocus.restore(origin);
  }, [drawerPresented, isAuthenticated]);
  // Options already disable inner native-stack animation while the sidebar is
  // presented. Hide the old root, commit its guarded swap, then reveal the
  // painted destination while closing the slab once.
  const selectDestination = (action: NavigationAction | null) => {
    if (
      !authenticatedNow.current ||
      !navigationRef.isReady() ||
      !action ||
      isNavigationBlocked()
    )
      return;
    if (!persistentSidebar && !drawerPresentedNow.current) {
      // The folded Duo's quick rail: no open sidebar to swap beneath or close,
      // so dispatch directly; native before-remove guards still apply.
      navigationRef.dispatch(action);
      return;
    }
    if (pendingDrawerNavigation.current || didNavigate.current) return;
    if (persistentSidebar) {
      // Native before-remove guards still own dirty editors and pending writes.
      navigationRef.dispatch(action);
      return;
    }
    pendingDrawerNavigation.current = action;
    if (workspaceDestinationMatches(navigationRef.getRootState(), action)) {
      didNavigate.current = true;
      revealContent();
      setDrawerOpen(false);
    } else {
      // Layout/focus events can be missed by a retained native screen. A failed
      // animation must never leave an authenticated app permanently invisible.
      selectionWatchdog.current = setTimeout(() => {
        if (pendingDrawerNavigation.current !== action) return;
        cancelSelection();
        setDrawerOpen(false);
      }, 2000);
      const dispatch = () => {
        if (pendingDrawerNavigation.current !== action) return;
        if (!authenticatedNow.current || isNavigationBlocked()) {
          cancelSelection();
          return;
        }
        navigationRef.dispatch(action);
      };
      if (contentTransition.current)
        contentTransition.current.fadeOut(dispatch);
      else dispatch();
    }
  };
  const onDrawerDismiss = useCallback(() => {
    cancelSelection();
    pendingMenuFocus.current = didNavigate.current
      ? undefined
      : drawerOrigin.current;
    didNavigate.current = false;
    drawerOrigin.current = undefined;
    setDrawerPresented(false);
  }, [cancelSelection]);
  const navigateFromDrawer = (route: WorkspaceRoute) => {
    if (
      !authenticatedNow.current ||
      !navigationRef.isReady() ||
      isNavigationBlocked()
    )
      return;
    if (route === "Assistant") return openChatFromDrawer({ kind: "home" });
    const action = workspaceNavigateAction(
      route,
      undefined,
      navigationRef.getRootState(),
    );
    // The root native modal sits above the whole push shell. Neither its
    // presentation nor its guarded dismissal changes the underlying offset.
    if (route === "AccountSettings") navigationRef.dispatch(action);
    else selectDestination(action);
  };
  const openChatFromDrawer = (target: WorkspaceChatTarget) => {
    if (navigationRef.isReady())
      selectDestination(
        workspaceChatNavigationAction(navigationRef.getRootState(), target),
      );
  };

  function revealSidebar() {
    if (
      !authenticatedNow.current ||
      accessibility.sheetOpen ||
      isNavigationBlocked()
    )
      return;
    Keyboard.dismiss();
    pendingMenuFocus.current = undefined;
    if (!drawerPresented)
      drawerOrigin.current = workspaceFocus.capture(accessibility.primaryKey);
    setDrawerPresented(true);
  }
  function toggleSidebar() {
    if (
      !authenticatedNow.current ||
      accessibility.sheetOpen ||
      isNavigationBlocked()
    )
      return;
    const showing = !persistentSidebarOpen;
    setPersistentSidebarOpen(showing);
    if (showing) {
      Keyboard.dismiss();
      workspaceFocus.releaseDrawer("persistent-workspace-sidebar");
      workspaceFocus.focusDrawerOnce("persistent-workspace-sidebar", () =>
        findNodeHandle(sidebarFocus.current),
      );
    }
  }
  function changeSidebar(open: boolean) {
    if (persistentSidebar) {
      if (
        open &&
        authenticatedNow.current &&
        !accessibility.sheetOpen &&
        !isNavigationBlocked()
      ) {
        Keyboard.dismiss();
        workspaceFocus.releaseDrawer("persistent-workspace-sidebar");
        workspaceFocus.focusDrawerOnce("persistent-workspace-sidebar", () =>
          findNodeHandle(sidebarFocus.current),
        );
      }
      return;
    }
    if (open) {
      if (
        !authenticatedNow.current ||
        accessibility.sheetOpen ||
        isNavigationBlocked() ||
        pendingDrawerNavigation.current ||
        didNavigate.current
      )
        return;
      revealSidebar();
    } else cancelSelection();
    setDrawerOpen(open);
  }

  const openQuickAction = (action: "scanner" | "terminal" | "recents") => {
    if (
      !authenticatedNow.current ||
      !navigationRef.isReady() ||
      isNavigationBlocked()
    )
      return;
    navigationRef.dispatch(
      action === "scanner"
        ? openScannerSheetAction()
        : action === "recents"
          ? openWorkspaceSheetAction("RecentChats")
          : openWorkspaceSheetAction("AgentKeyLogin", { mint: true }),
    );
  };
  // The Duo rail's quick-access entries open exactly what the side menu's
  // matching row opens; a pinned agent or crew opens its chat.
  const openQuickAccess = (item: QuickAccessItem) => {
    const open = (action: NavigationAction) => selectDestination(action);
    if (item.kind === "agent")
      return openChatFromDrawer({ kind: "agent", agentId: item.id });
    if (item.kind === "crew")
      return openChatFromDrawer({ kind: "group", groupId: item.id });
    switch (item.id) {
      case "new_chat":
        // Compose keeps the viewed agent, like the side menu's Chat button.
        return openChatFromDrawer({
          kind: "new",
          params: newAssistantChatParams(
            currentChat?.kind === "agent" ? currentChat.agentId : undefined,
          ),
        });
      case "notifications":
        return navigateFromDrawer("Notifications");
      case "scan_qr":
        return openQuickAction("scanner");
      case "terminal_login_code":
        return openQuickAction("terminal");
      case "recent_chats":
        return openQuickAction("recents");
      case "automations":
        return navigateFromDrawer("Automations");
      case "machines":
        return navigateFromDrawer("Machines");
      case "plugins":
        return navigateFromDrawer("Plugins");
      case "activity":
        return navigateFromDrawer("Activity");
      case "settings":
        return navigateFromDrawer("AccountSettings");
      case "create_agent":
      case "agents":
      case "crews":
        if (!navigationRef.isReady()) return;
        return open(
          item.id === "create_agent"
            ? workspaceNavigateAction(
                "AgentCreate",
                undefined,
                navigationRef.getRootState(),
              )
            : workspaceNavigateAction(
                "Agents",
                { tab: item.id === "crews" ? "groups" : "agents" },
                navigationRef.getRootState(),
              ),
        );
    }
  };
  const renderWorkspace = (children: ReactNode) => (
    <WorkspacePushShell
      contentTransition={contentTransition}
      enabled={isAuthenticated}
      visible={drawerOpen}
      presented={drawerPresented}
      obscured={accessibility.sheetOpen}
      gestureEnabled={
        !navigationBlocked &&
        !accessibility.sheetOpen &&
        (Boolean(edgeBackRevision && edgeBacks.current.size) ||
          isWorkspaceSidebarRoot(currentRouteName))
      }
      onEdgeBack={
        edgeBacks.current.size
          ? () => [...edgeBacks.current.values()].at(-1)?.()
          : undefined
      }
      onOpenChange={changeSidebar}
      sidebarCollapsed={sidebarCollapsed}
      sidebarRestored={sidebarRestored}
      onRevealStart={revealSidebar}
      onDismiss={onDrawerDismiss}
      initialFocusTarget={() => findNodeHandle(sidebarFocus.current)}
      sidebar={
        isAuthenticated ? (
          <WorkspaceDrawer
            visible={persistentSidebar || drawerPresented}
            disabled={navigationBlocked}
            initialFocusRef={sidebarFocus}
            currentRoute={currentRouteName}
            currentChat={currentChat}
            onOpenChat={openChatFromDrawer}
            onCreateAgent={() => {
              if (navigationRef.isReady())
                selectDestination(
                  workspaceNavigateAction(
                    "AgentCreate",
                    undefined,
                    navigationRef.getRootState(),
                  ),
                );
            }}
            onCreateGroup={() => {
              if (navigationRef.isReady())
                selectDestination(
                  workspaceNavigateAction(
                    "GroupEditor",
                    undefined,
                    navigationRef.getRootState(),
                  ),
                );
            }}
            onNavigate={navigateFromDrawer}
            onHome={() => navigateFromDrawer("Assistant")}
            onQuickAction={openQuickAction}
            onBrowse={(name) => {
              if (navigationRef.isReady())
                selectDestination(
                  workspaceNavigateAction(
                    name,
                    undefined,
                    navigationRef.getRootState(),
                  ),
                );
            }}
          />
        ) : null
      }
      quickRail={
        isAuthenticated
          ? (fullScreen, onToggleFullScreen, header) => (
              <WorkspaceQuickRail
                fullScreen={fullScreen}
                onToggleFullScreen={onToggleFullScreen}
                header={header}
                onOpen={openQuickAccess}
              />
            )
          : undefined
      }
    >
      <View collapsable={false} accessible={false} style={appRootStyles.fill}>
        {children}
      </View>
    </WorkspacePushShell>
  );
  return (
    <View style={appRootStyles.fill}>
      <NavigationContainer
        ref={navigationRef}
        linking={appLinking}
        onReady={() => {
          trackCurrentRoute();
          flushPendingNotificationNavigation();
        }}
        onStateChange={() => {
          trackCurrentRoute();
          flushPendingNotificationNavigation();
        }}
      >
        <StatusBar style={mode === "dark" ? "light" : "dark"} />
        <WorkspaceAccessibilityContext.Provider
          value={workspaceAccessibility}
        >
          <WorkspaceSidebarToggleContext.Provider
            value={{ persistent: persistentSidebar && cornerWindow, collapsed: sidebarCollapsed, }}
          >
          <AppNavigator
            onOpenMenu={() =>
              persistentSidebar && cornerWindow ? toggleSidebar() : changeSidebar(!drawerOpen)
            }
            onNavigationGuardChange={onNavigationGuardChange}
            onEdgeBackChange={onEdgeBackChange}
            onNavigationCancelled={() => {
              cancelSelection();
            }}
            onWorkspacePagePainted={(identity) => {
              if (!authenticatedNow.current || sessionEpoch !== getAuthEpoch())
                return;
              paintedPage.current = identity;
              if (
                authenticatedNow.current &&
                awaitingPaint.current === identity &&
                workspaceContentIdentity(navigationRef.getRootState()) ===
                  identity
              ) {
                revealContent();
                setDrawerOpen(false);
              }
            }}
            suppressWorkspaceAnimation={persistentSidebar || drawerPresented}
            renderWorkspace={renderWorkspace}
          />
          </WorkspaceSidebarToggleContext.Provider>
        </WorkspaceAccessibilityContext.Provider>
      </NavigationContainer>
      {/* ToastOverlay anchors to `top: 0` of its parent and relies on the
          screen's SafeAreaView for inset. Mounted globally it has no such
          parent, so give it explicit bounds inside the safe area: below the
          notch, and left of a corner status column (the Duo's cluster and
          rail). */}
      <View
        style={[
          pushToastStyles.overlay,
          { top: insets.top, left: insets.left, right: insets.right },
        ]}
        pointerEvents="box-none"
      >
        <ToastOverlay key={pushToastKey} toast={pushToast} duration={6000} />
      </View>
    </View>
  );
}

const appLoadingStyles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#07060e",
    justifyContent: "center",
    alignItems: "center",
    gap: 16,
  },
  text: {
    color: "#e8e4f0",
    fontSize: 16,
  },
});

const pushToastStyles = StyleSheet.create({
  overlay: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
  },
});

const appRootStyles = StyleSheet.create({
  fill: {
    flex: 1,
  },
});
