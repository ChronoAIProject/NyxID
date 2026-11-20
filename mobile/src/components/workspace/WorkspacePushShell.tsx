import { Platform } from "react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import {
  BackHandler,
  Pressable,
  StyleSheet,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  cancelAnimation,
  runOnJS,
  useAnimatedStyle,
  useAnimatedReaction,
  useSharedValue,
  withSpring,
  withTiming,
  Easing,
} from "react-native-reanimated";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { workspaceFocus } from "../../lib/accessibility/workspaceFocus";
import { useTheme } from "../../theme/ThemeContext";
import {
  clampSidebarOffset,
  settleSidebarOpen,
  SIDEBAR_EDGE_WIDTH,
  workspaceSidebarLayout,
} from "./workspacePushState";
import {
  FLOATING_TRAILING_INSET,
  workspaceCornerSensor,
  workspaceQuickRailAvailable,
  workspaceQuickRailFrame,
  workspaceStatusCluster,
} from "./workspaceLayout";
import {
  WorkspaceCornerSensorContext,
  WorkspaceQuickRailContext,
} from "./WorkspaceLayoutContext";
import { useQuickRailHidden } from "./workspaceQuickRailPreference";
import {
  WorkspaceFloatingInsetContext,
  WorkspaceRailMotionContext,
} from "./workspaceFloatingInset";
import {
  SLAB_MOTION_DURATION,
  SLAB_MOTION_EASING,
  sidebarInset,
} from "./workspaceSlabTransitions";
import { chatComposerFocus } from "../../features/assistant/chatComposerFocus";
import {
  WorkspaceColumnHeaderContext,
  type WorkspaceColumnHeaderControls,
} from "./workspaceColumnHeader";
import { useWorkspaceStatusOffset } from "./useWorkspaceStatusOffset";

export type WorkspaceContentTransitionHandle = {
  fadeOut: (onHidden: () => void) => void;
  reveal: () => void;
};

type Props = {
  contentTransition?: RefObject<WorkspaceContentTransitionHandle | null>;
  enabled: boolean;
  visible: boolean;
  presented: boolean;
  gestureEnabled: boolean;
  /** A native Settings modal owns input without changing the sidebar endpoint. */
  obscured?: boolean;
  onOpenChange: (open: boolean) => void;
  onRevealStart: () => void;
  onEdgeBack?: () => void;
  onDismiss: () => void;
  initialFocusTarget: () => number | null;
  sidebar: ReactNode;
  /** The Duo's rail (folded or unfolded, beside its status column); it
   * receives the Full screen action, whether full screen is on (which hides
   * its grouped shortcuts capsule; it stays mounted so its glass can dissolve
   * itself) and the focused chat's bar controls, which full screen moves into
   * the column. */
  quickRail?: (
    fullScreen: boolean,
    onToggleFullScreen: () => void,
    header: WorkspaceColumnHeaderControls | null,
  ) => ReactNode;
  /** Wide windows: the persistent sidebar is hidden and content takes the full width. */
  sidebarCollapsed?: boolean;
  /** The saved sidebar choice has loaded; later changes animate, the restore does not. */
  sidebarRestored?: boolean;
  children: ReactNode;
};

/** Keep the navigator mounted while switching between push and side-by-side. */
export function WorkspacePushShell(props: Props) {
  useAppLanguage();
  const { colors } = useTheme();
  const { width, height } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const windowMetrics = { platform: Platform.OS, width, height, rightInset: insets.right, };
  const [quickRailHidden, setQuickRailHidden] = useQuickRailHidden();
  const quickRailAvailable =
    props.enabled &&
    Boolean(props.quickRail) &&
    workspaceQuickRailAvailable(windowMetrics);
  const fullScreen = quickRailAvailable && quickRailHidden;
  const cornerSensor = workspaceCornerSensor(windowMetrics);
  // This posture's measured cluster (folded or unfolded): the rail starts
  // just below it.
  const cluster = workspaceStatusCluster(windowMetrics);
  const statusOffset = useWorkspaceStatusOffset(
    quickRailAvailable, width, height, insets.top,
  );
  // The focused chat's bar controls (registered while it is focused): full
  // screen moves them into the column below the restore capsule.
  const [columnHeader, setColumnHeader] = useState<{
    owner: symbol;
    controls: WorkspaceColumnHeaderControls;
  } | null>(null);
  const registerColumnHeader = useMemo(
    () => (owner: symbol, controls: WorkspaceColumnHeaderControls | null) =>
      setColumnHeader((current) =>
        controls
          ? { owner, controls }
          : current?.owner === owner
            ? null
            : current,
      ),
    [],
  );
  const layout = workspaceSidebarLayout(width);
  const persistent = props.enabled && layout.persistent;
  const persistentNow = useRef(persistent);
  persistentNow.current = persistent;
  const openWidth = layout.sidebarWidth;
  const visible = props.visible;
  const edgeBack = Boolean(props.onEdgeBack) && !visible;
  const reduced = useReducedMotion();
  const [settledOpen, setSettledOpen] = useState(false);
  const offset = useSharedValue(0);
  const dragOrigin = useSharedValue(0);
  const contentOpacity = useSharedValue(1);
  const contentChanging = useSharedValue(false);
  const fadeGeneration = useRef(0);
  const programmatic = useSharedValue(false);
  const dragActive = useSharedValue(false);
  const transition = useSharedValue(0);
  const sidebarCollapsed = persistent && cornerSensor && Boolean(props.sidebarCollapsed);
  // 1 = persistent sidebar shown, 0 = hidden (content full width).
  const sidebarShown = useSharedValue(sidebarCollapsed ? 0 : 1);
  // 0 = rail mode, 1 = full screen: one UI-thread clock for the content's
  // inset, the grouped capsule's fade and the status column's edge fade.
  const fullScreenProgress = useSharedValue(fullScreen ? 1 : 0);
  // Only a person's toggle animates; restoring the saved choice applies at once.
  const animateRail = useRef(false);
  const sidebarWasRestored = useRef(Boolean(props.sidebarRestored));
  const endpoint = useRef<string | undefined>(undefined);
  const generation = useRef(0);
  const dragging = useRef(false);
  const mounted = useRef(true);
  const focusOwner = useRef(Symbol("navigation-sidebar")).current;
  const acknowledgedClosed = useRef(false);
  const latest = useRef(props);
  latest.current = props;
  function restoreContent(fadeIn = false) {
    const hidden = contentChanging.value || contentOpacity.value < 1;
    fadeGeneration.current++;
    contentChanging.value = false;
    cancelAnimation(contentOpacity);
    contentOpacity.value =
      hidden && fadeIn && !reduced ? withTiming(1, { duration: 180 }) : 1;
  }
  function toggleFullScreen() {
    // The saved choice persists per device; the content takes (or returns)
    // the rail column in one smooth UI-thread transition.
    animateRail.current = true;
    setQuickRailHidden(!quickRailHidden);
  }
  const slabTiming = {
    duration: SLAB_MOTION_DURATION,
    easing: Easing.bezier(...SLAB_MOTION_EASING),
  };
  useEffect(() => {
    const target = fullScreen ? 1 : 0;
    // Reduce Motion swaps at once.
    if (animateRail.current && !reduced)
      fullScreenProgress.value = withTiming(target, slabTiming);
    else {
      cancelAnimation(fullScreenProgress);
      fullScreenProgress.value = target;
    }
    animateRail.current = false;
  }, [fullScreen, reduced]);
  const railMotion = useMemo(
    () =>
      quickRailAvailable
        ? {
            inset: FLOATING_TRAILING_INSET,
            fullScreen,
            progress: fullScreenProgress,
          }
        : null,
    [
      quickRailAvailable,
      fullScreen,
      fullScreenProgress
    ],
  );
  useEffect(() => {
    const target = sidebarCollapsed ? 0 : 1;
    if (sidebarWasRestored.current && persistent && !reduced)
      sidebarShown.value = withTiming(target, slabTiming);
    else {
      cancelAnimation(sidebarShown);
      sidebarShown.value = target;
    }
    sidebarWasRestored.current = Boolean(props.sidebarRestored);
  }, [sidebarCollapsed, persistent, reduced, props.sidebarRestored]);

  if (!props.presented || props.visible) acknowledgedClosed.current = false;
  useLayoutEffect(() => {
    workspaceFocus.setModalActive(
      focusOwner,
      props.enabled && !persistent && settledOpen && !props.obscured,
    );
  }, [focusOwner, props.enabled, persistent, settledOpen, props.obscured]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current++;
      cancelAnimation(offset);
      cancelAnimation(contentOpacity);
      fadeGeneration.current++;
      contentChanging.value = false;
      contentOpacity.value = 1;
      workspaceFocus.setModalActive(focusOwner, false);
    };
  }, [focusOwner, offset]);

  useLayoutEffect(() => {
    const controller = props.contentTransition;
    if (!controller) return;
    controller.current = {
      fadeOut(onHidden) {
        const id = ++fadeGeneration.current;
        contentChanging.value = true;
        cancelAnimation(contentOpacity);
        const complete = () => {
          if (
            mounted.current &&
            fadeGeneration.current === id &&
            latest.current.enabled
          )
            onHidden();
        };
        if (reduced) {
          contentOpacity.value = 1;
          complete();
        } else {
          const recover = () => {
            if (mounted.current && fadeGeneration.current === id)
              restoreContent();
          };
          contentOpacity.value = withTiming(0, { duration: 120 }, (done) => {
            "worklet";
            if (done) runOnJS(complete)();
            else runOnJS(recover)();
          });
        }
      },
      reveal() {
        restoreContent(true);
      },
    };
    return () => {
      controller.current = null;
      restoreContent();
    };
  }, [props.contentTransition, reduced, contentOpacity]);

  function reconcileEndpoint(open: boolean) {
    const state = latest.current;
    // Read the live UI endpoint as well as the current controlled request.
    // A cancelled/zero-distance drag can reach an endpoint without the original
    // timing callback; an obsolete callback cannot acknowledge a newer drag.
    if (
      !mounted.current ||
      !state.enabled ||
      persistentNow.current ||
      dragging.current ||
      dragActive.value ||
      programmatic.value ||
      state.visible !== open ||
      Math.abs(offset.value - (open ? openWidth : 0)) > 0.5
    )
      return;
    if (!open) restoreContent();
    setSettledOpen(open);
    if (!open && state.presented && !acknowledgedClosed.current) {
      acknowledgedClosed.current = true;
      state.onDismiss();
    }
  }
  function finish(id: number, open: boolean) {
    if (id === generation.current) reconcileEndpoint(open);
  }
  useAnimatedReaction(
    () => {
      if (programmatic.value || dragActive.value) return null;
      if (Math.abs(offset.value) < 0.5) return false;
      if (Math.abs(offset.value - openWidth) < 0.5) return true;
      return null;
    },
    (open, previous) => {
      if (open !== null && open !== previous) runOnJS(reconcileEndpoint)(open);
    },
    [openWidth],
  );
  useEffect(() => {
    if (!props.visible && contentChanging.value) restoreContent();
    reconcileEndpoint(props.visible);
  }, [props.visible, props.presented]);
  useEffect(() => {
    if (!settledOpen || !props.enabled || persistent || props.obscured) return;
    workspaceFocus.focusModalOnce(focusOwner, () =>
      latest.current.visible && !latest.current.obscured
        ? latest.current.initialFocusTarget()
        : null,
    );
  }, [settledOpen, props.enabled, persistent, props.obscured, focusOwner]);
  function animate(open: boolean, fingerRelease = false) {
    endpoint.current = `${open}:${openWidth}:${reduced}`;
    const id = ++generation.current;
    const target = open ? openWidth : 0;
    cancelAnimation(offset);
    transition.value = id;
    programmatic.value = !reduced;
    if (reduced) {
      offset.value = target;
      // The controlled visibility/AX commit must precede focus or navigation.
      requestAnimationFrame(() => finish(id, open));
    } else {
      const complete = (done?: boolean) => {
        "worklet";
        if (transition.value !== id) return;
        programmatic.value = false;
        if (done) runOnJS(finish)(id, open);
      };
      offset.value = fingerRelease
        ? withSpring(
            target,
            { stiffness: 300, damping: 30, mass: 0.8, overshootClamping: true },
            complete,
          )
        : withTiming(
            target,
            { duration: 300, easing: Easing.bezier(0.2, 0.8, 0.2, 1) },
            complete,
          );
    }
  }
  useEffect(() => {
    if (!props.enabled || persistent) {
      generation.current++;
      transition.value = generation.current;
      cancelAnimation(offset);
      offset.value = 0;
      if (!props.enabled) restoreContent();
      setSettledOpen(false);
      dragging.current = false;
      dragActive.value = false;
      programmatic.value = false;
      endpoint.current = undefined;
      return;
    }
    const next = `${props.visible}:${openWidth}:${reduced}`;
    if (!dragging.current && endpoint.current !== next) {
      if (!props.presented && !props.visible && offset.value === 0)
        endpoint.current = next;
      else animate(props.visible);
    }
  }, [props.visible, props.enabled, persistent, openWidth, reduced]);

  function revealStart() {
    if (
      !latest.current.enabled ||
      latest.current.obscured ||
      (persistentNow.current && !latest.current.onEdgeBack) ||
      !(latest.current.visible || latest.current.gestureEnabled)
    )
      return;
    generation.current++;
    dragging.current = true;
    if (!latest.current.onEdgeBack || latest.current.visible)
      latest.current.onRevealStart();
  }
  function finishBack(translation: number, velocity: number) {
    if (!mounted.current || !dragging.current || !latest.current.enabled)
      return;
    dragging.current = false;
    if (translation > 40 || (translation > 8 && velocity > 500))
      latest.current.onEdgeBack?.();
  }
  function settle(open: boolean) {
    if (!mounted.current || !latest.current.enabled || !dragging.current)
      return;
    dragging.current = false;
    latest.current.onOpenChange(open);
    animate(open, true);
  }
  useEffect(() => {
    if (!props.enabled || persistent || !props.presented || props.obscured)
      return;
    const subscription = BackHandler.addEventListener(
      "hardwareBackPress",
      () => {
        latest.current.onOpenChange(false);
        return true;
      },
    );
    return () => subscription.remove();
  }, [props.enabled, persistent, props.presented, props.obscured]);

  // These detectors live on dedicated edge/sliver targets, never above a
  // transcript, code/table ScrollView or composer in the responder hierarchy.
  const pan = Gesture.Pan()
    .enabled(
      props.enabled &&
        !props.obscured &&
        (persistent
          ? edgeBack && props.gestureEnabled
          : visible
            ? props.presented
            : props.gestureEnabled),
    )
    .activeOffsetX(visible ? -18 : 18)
    .failOffsetY([-10, 10])
    .onTouchesDown((_event, manager) => {
      if (programmatic.value || contentChanging.value) manager.fail();
    })
    .onStart(() => {
      if (programmatic.value || contentChanging.value) return;
      dragActive.value = true;
      cancelAnimation(offset);
      dragOrigin.value = offset.value;
      runOnJS(revealStart)();
    })
    .onUpdate((event) => {
      if (dragActive.value && !edgeBack && !reduced)
        offset.value = clampSidebarOffset(
          dragOrigin.value + event.translationX,
          openWidth,
        );
    })
    .onEnd((event) => {
      if (!dragActive.value) return;
      dragActive.value = false;
      if (edgeBack) {
        runOnJS(finishBack)(event.translationX, event.velocityX);
        return;
      }
      const end = reduced
        ? clampSidebarOffset(dragOrigin.value + event.translationX, openWidth)
        : offset.value;
      runOnJS(settle)(settleSidebarOpen(end, event.velocityX, openWidth));
    })
    .onFinalize((_event, success) => {
      if (!success && dragActive.value) {
        dragActive.value = false;
        if (edgeBack) runOnJS(finishBack)(0, 0);
        else runOnJS(settle)(visible);
      }
    });
  const slabStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: persistent ? 0 : offset.value }],
    // Wide windows: the slab's left edge follows the sidebar as it hides or
    // shows, so content width changes smoothly with it.
    marginLeft: persistent ? sidebarInset(openWidth, sidebarShown.value) : 0,
    borderTopLeftRadius: persistent
      ? 16 * sidebarShown.value
      : offset.value > 0
        ? 16
        : 0,
    borderBottomLeftRadius: persistent
      ? 16 * sidebarShown.value
      : offset.value > 0
        ? 16
        : 0,
    shadowOpacity: !persistent && offset.value > 0 ? 0.2 : 0,
  }));
  const clipStyle = useAnimatedStyle(() => ({
    opacity: contentOpacity.value,
    borderTopLeftRadius: persistent || offset.value > 0 ? 16 : 0,
    borderBottomLeftRadius: persistent || offset.value > 0 ? 16 : 0,
  }));
  const active = props.enabled && !persistent && props.presented;
  const sidebarInteractive =
    props.enabled &&
    (persistent ? !sidebarCollapsed : settledOpen) &&
    !props.obscured;
  const sidebarModal = !persistent && sidebarInteractive;
  return (
    <View
      style={[styles.root, { backgroundColor: colors.sidebarBase }]}
      pointerEvents={props.obscured ? "none" : "auto"}
      accessibilityElementsHidden={Boolean(props.obscured)}
      importantForAccessibility={
        props.obscured ? "no-hide-descendants" : "auto"
      }
      onAccessibilityEscape={() => {
        if (sidebarModal) props.onOpenChange(false);
      }}
    >
      {props.enabled ? (
        <View
          testID="workspace-sidebar-base"
          accessible={false}
          accessibilityElementsHidden={!sidebarInteractive}
          importantForAccessibility={
            sidebarInteractive ? "auto" : "no-hide-descendants"
          }
          pointerEvents={sidebarInteractive ? "auto" : "none"}
          style={[styles.sidebar, { width: openWidth }]}
        >
          {props.sidebar}
        </View>
      ) : null}
      <Animated.View
        testID="workspace-content-slab"
        style={[
          styles.slab,
          {
            backgroundColor: colors.bg,
            shadowColor: colors.shadowColor,
          },
          slabStyle,
        ]}
      >
        <Animated.View
          testID="workspace-slab-content"
          accessible={false}
          accessibilityElementsHidden={sidebarModal}
          importantForAccessibility={
            sidebarModal ? "no-hide-descendants" : "auto"
          }
          style={[styles.content, clipStyle]}
        >
          <View
            testID="workspace-slab-body"
            style={styles.content}
            // Observes every touch in the slab (pages, headers, rail, sheets)
            // without claiming it: one that began outside a focused chat
            // composer blurs it when it ends.
            onTouchStart={
              cornerSensor ? chatComposerFocus.workspaceTouchStart : undefined
            }
            onTouchEnd={
              cornerSensor ? chatComposerFocus.workspaceTouchEnd : undefined
            }
          >
            <WorkspaceCornerSensorContext.Provider value={cornerSensor}>
              {/* The header keeps the same controls in rail mode and full
                  screen: on the Duo (folded or unfolded, narrow or beside
                  the persistent sidebar) the rail carries New chat,
                  Notifications, Scan QR, Recent chats and Settings. */}
              <WorkspaceQuickRailContext.Provider value={quickRailAvailable}>
                {/* Rail mode: content keeps clear of the rail column. Full
                    screen: it runs to the 12pt margin (animated). */}
                <WorkspaceRailMotionContext.Provider value={railMotion}>
                  <WorkspaceFloatingInsetContext.Provider
                    value={railMotion && !fullScreen ? railMotion.inset : 0}
                  >
                    <WorkspaceColumnHeaderContext.Provider
                      value={registerColumnHeader}
                    >
                      {props.children}
                    </WorkspaceColumnHeaderContext.Provider>
                  </WorkspaceFloatingInsetContext.Provider>
                </WorkspaceRailMotionContext.Provider>
              </WorkspaceQuickRailContext.Provider>
            </WorkspaceCornerSensorContext.Provider>
          </View>
        </Animated.View>
        {props.enabled &&
        !props.obscured &&
        (persistent
          ? edgeBack && props.gestureEnabled
          : active || props.gestureEnabled) ? (
          <GestureDetector gesture={pan}>
            <Pressable
              testID={
                active
                  ? "workspace-sidebar-close-sliver"
                  : "workspace-sidebar-reveal-edge"
              }
              accessible={active}
              focusable={active}
              accessibilityElementsHidden={!active}
              importantForAccessibility={active ? "yes" : "no-hide-descendants"}
              accessibilityRole={active ? "button" : undefined}
              accessibilityLabel={
                active ? translateApp("Close navigation") : undefined
              }
              onPress={active ? () => props.onOpenChange(false) : undefined}
              style={active ? StyleSheet.absoluteFill : styles.edge}
            />
          </GestureDetector>
        ) : null}
      </Animated.View>
      {quickRailAvailable ? (
        // Outside the sliding slab: the rail stays fixed on the status
        // column while the folded drawer slides the page aside (it sits
        // over the slab's visible edge). The slab spans the window's right
        // edge in every posture, so the same right-anchored frame applies.
        // It observes touches like the slab body so a tap on it still
        // blurs a focused composer.
        <WorkspaceRailMotionContext.Provider value={railMotion}>
          <View
            testID="workspace-quick-rail"
            pointerEvents="box-none"
            onTouchStart={
              cornerSensor ? chatComposerFocus.workspaceTouchStart : undefined
            }
            onTouchEnd={
              cornerSensor ? chatComposerFocus.workspaceTouchEnd : undefined
            }
            style={[
              styles.quickRailHost,
              workspaceQuickRailFrame(insets.top, insets.bottom, cluster, statusOffset),
            ]}
          >
            {props.quickRail!(
              fullScreen,
              toggleFullScreen,
              columnHeader?.controls ?? null,
            )}
          </View>
        </WorkspaceRailMotionContext.Provider>
      ) : null}
    </View>
  );
}
const styles = StyleSheet.create({
  root: { flex: 1, overflow: "hidden" },
  edge: {
    position: "absolute",
    left: 0,
    top: 0,
    bottom: 0,
    width: SIDEBAR_EDGE_WIDTH,
  },
  sidebar: { position: "absolute", top: 0, bottom: 0, left: 0 },
  // The rail sits in the strip the body keeps clear while it is shown,
  // centered on the status cluster's column.
  quickRailHost: { position: "absolute", alignItems: "center" },
  slab: {
    flex: 1,
    shadowOffset: { width: -3, height: 0 },
    shadowRadius: 12,
    elevation: 2,
  },
  content: { flex: 1, overflow: "hidden" },

});
