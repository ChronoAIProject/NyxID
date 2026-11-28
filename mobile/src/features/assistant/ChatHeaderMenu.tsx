import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from "react";
import {
  AccessibilityInfo,
  Animated,
  Easing,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  findNodeHandle,
  useWindowDimensions,
} from "react-native";
import type { LucideIcon } from "lucide-react-native";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import { FloatingCapsule } from "../../components/FloatingControl";
import { FLOATING_CONTROL_SIZE } from "../../components/floatingControlTokens";
import { FLOATING_SURFACE_SHADOW } from "../../components/workspace/workspaceFloatingSurface";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

const MENU_WIDTH = 240;
const MENU_MARGIN = 8;
const MENU_ROW_HEIGHT = 48;

export type ChatMenuOption = {
  id: string;
  label: string;
  accessibilityLabel?: string;
  Icon: LucideIcon;
  badge?: string;
  badgeColor?: string;
  disabled?: boolean;
  onPress: () => void;
};

/** A copy of the measured capsule grows in the modal, then returns to its anchor. */
export function ChatHeaderMenu({
  scope,
  disabled,
  options,
  controls,
}: {
  scope: string;
  disabled?: boolean;
  options: ChatMenuOption[];
  controls: (open: () => void, trigger: Ref<View>) => ReactNode;
}) {
  const { t } = useAppLanguage();
  const { colors, mode } = useTheme();
  const { width, height, fontScale } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const reducedMotion = useReducedMotion();
  const anchor = useRef<View>(null);
  const trigger = useRef<View>(null);
  const firstOption = useRef<View>(null);
  const generation = useRef(0);
  const closing = useRef(false);
  const pendingAction = useRef<
    { id: string; generation: number; revision: number } | undefined
  >(undefined);
  const dismissRequest = useRef<number | undefined>(undefined);
  const optionShape = JSON.stringify(
    options.map(({ id, disabled }) => [id, !!disabled]),
  );
  const authority = useRef({ shape: optionShape, revision: 0 });
  if (authority.current.shape !== optionShape) {
    authority.current = {
      shape: optionShape,
      revision: authority.current.revision + 1,
    };
  }
  const live = useRef({ scope, disabled, options });
  live.current = { scope, disabled, options };
  const restoreFocus = useRef(false);
  const progress = useRef(new Animated.Value(0)).current;
  const animation = useRef<Animated.CompositeAnimation | undefined>(undefined);
  const animationRevision = useRef(0);
  const [placement, setPlacement] = useState<{
    generation: number;
    top: number;
    right: number;
    width: number;
    height: number;
  }>();
  const [contentHeight, setContentHeight] = useState(0);
  const [overlayHeight, setOverlayHeight] = useState(0);
  const [presented, setPresented] = useState(false);

  useLayoutEffect(() => {
    setPlacement(undefined);
    closing.current = false;
    return () => {
      generation.current++;
      animationRevision.current++;
      animation.current?.stop();
      pendingAction.current = undefined;
      dismissRequest.current = undefined;
      restoreFocus.current = false;
    };
  }, [scope, disabled, width, height, fontScale, insets.right, insets.top]);

  function focus(view: View | null) {
    const handle = view && findNodeHandle(view);
    if (handle) AccessibilityInfo.setAccessibilityFocus(handle);
  }

  function animate(toValue: number, complete: () => void) {
    const revision = ++animationRevision.current;
    animation.current?.stop();
    if (reducedMotion) {
      progress.setValue(toValue);
      complete();
      return;
    }
    animation.current = Animated.timing(progress, {
      toValue,
      duration: toValue ? 220 : 180,
      easing: Easing.out(Easing.cubic),
      useNativeDriver: false,
    });
    animation.current.start(({ finished }) => {
      if (finished && revision === animationRevision.current) complete();
    });
  }

  function openMenu() {
    if (disabled || placement || closing.current || pendingAction.current)
      return;
    const request = ++generation.current;
    anchor.current?.measureInWindow((x, y, anchorWidth, anchorHeight) => {
      if (
        request !== generation.current ||
        anchorWidth <= 0 ||
        anchorHeight <= 0
      )
        return;
      closing.current = false;
      setPresented(false);
      progress.setValue(0);
      setContentHeight(0);
      setOverlayHeight(0);
      setPlacement({
        generation: request,
        top: y,
        right: width - x - anchorWidth,
        width: anchorWidth,
        height: anchorHeight,
      });
    });
  }

  function closeMenu(optionId?: string) {
    if (
      !placement ||
      closing.current ||
      placement.generation !== generation.current
    )
      return;
    const option = optionId
      ? live.current.options.find(({ id }) => id === optionId)
      : undefined;
    if (live.current.disabled || (optionId && (!option || option.disabled)))
      return;
    closing.current = true;
    dismissRequest.current = placement.generation;
    pendingAction.current = option
      ? {
          id: option.id,
          generation: placement.generation,
          revision: authority.current.revision,
        }
      : undefined;
    restoreFocus.current = !option;
    animate(0, () => setPlacement(undefined));
  }

  function finishDismissal(request: number | undefined) {
    if (request === undefined || request !== dismissRequest.current) return;
    dismissRequest.current = undefined;
    const action = pendingAction.current;
    pendingAction.current = undefined;
    closing.current = false;
    if (!live.current.disabled && request === generation.current) {
      const option =
        action && live.current.options.find(({ id }) => id === action.id);
      if (
        action &&
        action.generation === request &&
        action.revision === authority.current.revision &&
        option &&
        !option.disabled
      )
        option.onPress();
      else if (!action && restoreFocus.current) focus(trigger.current);
    }
    restoreFocus.current = false;
  }

  // Android has no native Modal onDismiss event. This runs after the closed
  // modal has committed; iOS waits for UIKit before opening another sheet.
  useEffect(() => {
    if (!placement && Platform.OS !== "ios")
      finishDismissal(dismissRequest.current);
  }, [placement]);

  useEffect(() => {
    if (!reducedMotion || !placement) return;
    animationRevision.current++;
    animation.current?.stop();
    progress.setValue(closing.current ? 0 : 1);
    if (closing.current) setPlacement(undefined);
  }, [reducedMotion, placement, progress]);

  const menu = disabled ? undefined : placement;
  const modalRequest = menu?.generation ?? dismissRequest.current;
  const menuWidth = menu
    ? Math.min(MENU_WIDTH, width - menu.right - insets.left - MENU_MARGIN)
    : 0;
  const menuHeight = menu
    ? Math.min(
        contentHeight ||
          options.length * MENU_ROW_HEIGHT * Math.max(1, fontScale),
        Math.max(
          0,
          (overlayHeight || height) - menu.top - insets.bottom - MENU_MARGIN,
        ),
      )
    : 0;

  return (
    <>
      <View
        ref={anchor}
        collapsable={false}
        testID="chat-header-actions"
        style={[
          styles.actions,
          menu && { width: menu.width, height: menu.height },
        ]}
      >
        {!menu || !presented ? (
          <FloatingCapsule style={styles.controls}>
            {controls(openMenu, trigger)}
          </FloatingCapsule>
        ) : null}
      </View>
      <Modal
        transparent
        visible={!!menu}
        animationType="none"
        statusBarTranslucent
        navigationBarTranslucent
        onRequestClose={() => closeMenu()}
        onDismiss={() => finishDismissal(modalRequest)}
        onShow={() => {
          if (menu && menu.generation === generation.current) {
            setPresented(true);
            if (!closing.current) animate(1, () => focus(firstOption.current));
          }
        }}
      >
        {menu ? (
          <SafeAreaView
            edges={Platform.OS === "android" ? ["top"] : []}
            style={styles.overlay}
          >
            <View
              style={styles.overlay}
              accessibilityViewIsModal
              onAccessibilityEscape={() => closeMenu()}
              onLayout={({ nativeEvent }) =>
                setOverlayHeight(nativeEvent.layout.height)
              }
            >
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("Close menu")}
                style={StyleSheet.absoluteFill}
                onPress={() => closeMenu()}
              />
              <Animated.View
                testID="chat-header-menu"
                accessibilityRole="menu"
                onAccessibilityEscape={() => closeMenu()}
                style={[
                  styles.menu,
                  mode === "light" && FLOATING_SURFACE_SHADOW,
                  {
                    backgroundColor: colors.menuFill,
                    borderColor: colors.border,
                    shadowColor: colors.shadowColor,
                    top: menu.top,
                    right: menu.right,
                    width: progress.interpolate({
                      inputRange: [0, 1],
                      outputRange: [menu.width, menuWidth],
                    }),
                    height: progress.interpolate({
                      inputRange: [0, 1],
                      outputRange: [menu.height, menuHeight],
                    }),
                  },
                ]}
              >
                <FloatingCapsule style={styles.surface}>
                  <Animated.View
                    pointerEvents="none"
                    accessibilityElementsHidden
                    importantForAccessibility="no-hide-descendants"
                    style={[
                      styles.source,
                      {
                        width: menu.width,
                        height: menu.height,
                        opacity: progress.interpolate({
                          inputRange: [0, 1],
                          outputRange: [1, 0],
                        }),
                      },
                    ]}
                  >
                    {controls(() => {}, null)}
                  </Animated.View>
                  <Animated.View
                    style={[
                      styles.options,
                      {
                        width: menuWidth,
                        height: menuHeight,
                        opacity: progress.interpolate({
                          inputRange: [0, 0.3, 1],
                          outputRange: [0, 0, 1],
                        }),
                      },
                    ]}
                  >
                    <ScrollView
                      bounces={false}
                      showsVerticalScrollIndicator={false}
                      onContentSizeChange={(_width, measuredHeight) =>
                        setContentHeight(measuredHeight)
                      }
                      keyboardShouldPersistTaps="always"
                    >
                      {options.map((option, index) => (
                        <Pressable
                          key={option.id}
                          ref={index === 0 ? firstOption : undefined}
                          accessibilityRole="menuitem"
                          accessibilityLabel={
                            option.accessibilityLabel ?? option.label
                          }
                          disabled={option.disabled}
                          accessibilityState={{ disabled: !!option.disabled }}
                          onAccessibilityEscape={() => closeMenu()}
                          onPress={() => closeMenu(option.id)}
                          style={({ pressed }) => [
                            styles.row,
                            index > 0 && {
                              borderTopWidth: StyleSheet.hairlineWidth,
                              borderTopColor: colors.borderSoft,
                            },
                            pressed && { backgroundColor: colors.navActive },
                            option.disabled && styles.disabled,
                          ]}
                        >
                          <option.Icon size={19} color={colors.textPrimary} />
                          <Text
                            style={[styles.label, { color: colors.textPrimary }]}
                          >
                            {option.label}
                          </Text>
                          {option.badge ? (
                            <Text
                              numberOfLines={1}
                              style={[styles.badge, { color: option.badgeColor }]}
                            >
                              {option.badge}
                            </Text>
                          ) : null}
                        </Pressable>
                      ))}
                    </ScrollView>
                  </Animated.View>
                </FloatingCapsule>
              </Animated.View>
            </View>
          </SafeAreaView>
        ) : null}
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  actions: { flexShrink: 0, minHeight: FLOATING_CONTROL_SIZE },
  controls: { flexDirection: "row", alignItems: "center" },
  overlay: { flex: 1 },
  // Elevation belongs outside the clipped capsule so its soft edge stays visible.
  menu: {
    position: "absolute",
    borderRadius: FLOATING_CONTROL_SIZE / 2,
    borderWidth: 1,
  },
  surface: { flex: 1, overflow: "hidden" },
  source: {
    position: "absolute",
    top: 0,
    right: 0,
    flexDirection: "row",
    alignItems: "center",
  },
  options: { position: "absolute", top: 0, right: 0 },
  row: {
    minHeight: MENU_ROW_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  label: { flex: 1, fontFamily: fonts.bodyRegular, fontSize: 16 },
  badge: { fontFamily: fonts.bodySemi, fontSize: 13 },
  disabled: { opacity: 0.4 },
});
