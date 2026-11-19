import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AppState,
  findNodeHandle,
  ScrollView,
  StyleSheet,
  View,
  type LayoutChangeEvent,
} from "react-native";
import { ChevronLeft, Menu, MoreHorizontal } from "lucide-react-native";
import { AgentAvatar, AgentAvatarStack } from "../AgentAvatar";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { useNotificationIndicator } from "../../features/notifications/useNotifications";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useQuickAccess } from "../../features/quickAccess/useQuickAccess";
import { useQuickAccessSources } from "../../features/quickAccess/useQuickAccessSources";
import {
  resolveQuickAccess,
  type QuickAccessEntry,
} from "../../features/quickAccess/quickAccessModel";
import { QuickAccessGlyph } from "../../features/quickAccess/QuickAccessGlyph";
import type { QuickAccessItem } from "../../lib/api/quickAccessSchema";
import {
  QUICK_RAIL_BUTTON_GAP,
  QUICK_RAIL_SECTION_GAP,
  QUICK_RAIL_TAB_SIZE,
  QUICK_RAIL_WIDTH,
  workspaceQuickRailShortcutsViewport,
} from "./workspaceLayout";
import {
  FloatingButton,
  FloatingCapsule,
  FloatingGroup,
  FloatingItem,
  floatingGlassAvailable,
  useFloatingSurfaceColor,
} from "../FloatingControl";
import {
  HEADER_ACTION_ENTERING,
  HEADER_ACTION_EXITING,
  MotionView,
  RailShortcutIcon,
  RailShortcutsView,
} from "./WorkspaceSlabMotion";
import { ExpandHorizontalIcon, RestoreHorizontalIcon } from "./HorizontalArrowIcons";
import type { WorkspaceColumnHeaderControls } from "./workspaceColumnHeader";

export type WorkspaceQuickRailActions = {
  /** Open one quick-access entry (a destination, or a pinned agent or crew). */
  onOpen: (item: QuickAccessItem) => void;
};

/** One complete phrase per state (the header bell's), never assembled. */
export function quickRailNotificationsLabel(
  t: (phrase: string, values?: Record<string, string | number>) => string,
  unread: number,
) {
  return unread > 0
    ? t("Notifications, {{unread}} unread", { unread })
    : t("Notifications");
}

const CAPSULE_RADIUS = QUICK_RAIL_TAB_SIZE / 2;

/**
 * The Duo's floating controls (folded or unfolded, narrow or beside the
 * persistent sidebar) below the sensor cluster, in the shared
 * floating-control style (the header buttons' surface): the full-screen
 * toggle in its own small capsule at the shared toggle position (Full screen,
 * or Show quick actions in full screen: the same control, so toggling never
 * moves it under the finger); then one vertical capsule holding the person's
 * quick-access entries (saved to the account, edited in Settings › Quick
 * access; by default New chat, Notifications, Scan QR, Recent chats and
 * Settings) as plain icon buttons, scrolling inside its glass when they do
 * not fit; then, on a chat, its bar's controls (Menu or Back, the agent,
 * More) in one capsule (`header`), since the Duo's chats have no top bar in
 * rail mode; all in one glass container, 12pt apart. Full screen hides the
 * shortcuts and the header capsule (the chat's own bar returns) while the
 * content takes the rail column; the toggle stays where it is and floats
 * over the content. The drawer keeps the full workspace.
 */
export function WorkspaceQuickRail({
  onOpen,
  fullScreen,
  onToggleFullScreen,
  header,
}: WorkspaceQuickRailActions & {
  fullScreen: boolean;
  onToggleFullScreen: () => void;
  /** The focused chat's bar controls (none on other pages). */
  header?: WorkspaceColumnHeaderControls | null;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const surface = useFloatingSurfaceColor();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  // The same flag check and summary the sidebar's Notifications row uses.
  const notifications = useNotificationIndicator(foreground && isConnected);
  // The account's list (the default until it loads), with pinned agents and
  // crews resolved from the side menu's lists; missing ones are skipped.
  const { items } = useQuickAccess().state;
  const sources = useQuickAccessSources({
    agents: items.some((item) => item.kind === "agent"),
    crews: items.some((item) => item.kind === "crew"),
  });
  const entries = resolveQuickAccess(items, {
    ...sources,
    notificationsEnabled: notifications.enabled,
  });
  // The column's height bounds the shortcuts capsule (down to the
  // composer's bottom); it scrolls only when its entries do not fit.
  const [columnHeight, setColumnHeight] = useState<number>();
  const measure = ({ nativeEvent }: LayoutChangeEvent) =>
    setColumnHeight(nativeEvent.layout.height);
  const viewport = workspaceQuickRailShortcutsViewport(
    entries.length,
    columnHeight,
    Boolean(header) && !fullScreen,
  );
  const shortcuts = !fullScreen;
  const shortcutItems = entries.map((entry) => (
    <RailItem
      key={entry.key}
      label={
        entry.kind === "destination" && entry.id === "notifications"
          ? quickRailNotificationsLabel(t, notifications.unread)
          : entry.label
      }
      onPress={() => onOpen(entry.item)}
      visible={shortcuts}
    >
      <RailGlyph
        entry={entry}
        unread={notifications.unread}
        color={colors.textPrimary}
        surface={surface}
        dot={{ backgroundColor: colors.primary, borderColor: colors.bg }}
      />
    </RailItem>
  ));
  return (
    <View
      testID="quick-rail-column"
      pointerEvents="box-none"
      onLayout={measure}
      style={styles.column}
    >
      {/* One glass container: its capsules render as Apple's grouped glass. */}
      <FloatingGroup
        testID="workspace-quick-rail-content"
        accessibilityRole="toolbar"
        accessibilityLabel={t("Quick actions")}
        spacing={QUICK_RAIL_BUTTON_GAP}
        pointerEvents="box-none"
        style={styles.rail}
      >
        {/* The toggle: the same floating button as every floating control,
            in a capsule of the grouped capsule's glass, width and corner
            shape. Expand and restore cross-fade inside it. */}
        <FloatingButton
          testID="quick-rail-expand"
          accessibilityLabel={fullScreen ? t("Show quick actions") : t("Full screen")}
          onPress={onToggleFullScreen}
          capsuleStyle={styles.capsule}
          // The rail's clear glass; nearly solid in full screen over content.
          capsuleGlass={fullScreen ? "railOpaque" : "rail"}
        >
          <MotionView
            key={fullScreen ? "restore" : "expand"}
            entering={HEADER_ACTION_ENTERING}
            exiting={HEADER_ACTION_EXITING}
          >
            {/* The icon shows the current state: rail mode the inward pair
                (content narrowed), full screen the outward pair (expanded). */}
            {fullScreen ? (
              <ExpandHorizontalIcon size={20} color={colors.textPrimary} />
            ) : (
              <RestoreHorizontalIcon size={20} color={colors.textPrimary} />
            )}
          </MotionView>
        </FloatingButton>
        {/* Full screen hides the grouped capsule with the content's inset:
            its glass dissolves natively and its icons fade (a glass capsule
            never sits under a fading ancestor; only the fallback surface
            fades with this wrapper). Hidden, it takes no touches and leaves
            VoiceOver. An empty list shows no capsule. */}
        {entries.length > 0 ? (
          <RailShortcutsView
            testID="quick-rail-group-motion"
            fadeOpacity={!floatingGlassAvailable()}
            pointerEvents={shortcuts ? "box-none" : "none"}
            accessibilityElementsHidden={!shortcuts}
            importantForAccessibility={shortcuts ? "auto" : "no-hide-descendants"}
          >
            <FloatingCapsule
              testID="quick-rail-group"
              glass="rail"
              visible={shortcuts}
              style={[
                styles.capsule,
                styles.group,
                viewport.scrolls ? { height: viewport.height } : null,
              ]}
            >
              {viewport.scrolls ? (
                // More entries than fit: they scroll inside the glass, which
                // stays put; no scroll indicator over the icons.
                <ScrollView
                  testID="quick-rail-group-scroll"
                  showsVerticalScrollIndicator={false}
                  style={styles.scroll}
                  contentContainerStyle={styles.group}
                >
                  {shortcutItems}
                </ScrollView>
              ) : (
                shortcutItems
              )}
            </FloatingCapsule>
          </RailShortcutsView>
        ) : null}
        {/* On a chat: its bar's controls below the shortcuts (rail mode;
            full screen hides both and the chat's bar returns). */}
        {header ? (
          <ColumnHeaderCapsule controls={header} shown={!fullScreen} />
        ) : null}
      </FloatingGroup>
    </View>
  );
}

/** A shortcut's mark: the bell keeps its unread dot inside the capsule. */
function RailGlyph({
  entry,
  unread,
  color,
  surface,
  dot,
}: {
  entry: QuickAccessEntry;
  unread: number;
  color: string;
  surface: string;
  dot: { backgroundColor: string; borderColor: string };
}) {
  const glyph = <QuickAccessGlyph entry={entry} color={color} surface={surface} />;
  if (entry.kind !== "destination" || entry.id !== "notifications" || unread <= 0)
    return glyph;
  return (
    <View>
      {glyph}
      <View testID="quick-rail-notifications-dot" style={[styles.dot, dot]} />
    </View>
  );
}

/**
 * A chat's bar controls on the Duo, whose chats have no top bar: Menu (or
 * Back), the agent (the identity pill's picker) and More in one capsule.
 * Rail mode: a vertical capsule in the column below the shortcuts.
 * Full screen: the chat's own top bar returns (as on an iPhone) and this
 * capsule fades out with the shortcuts; hidden, it takes no touches and
 * leaves VoiceOver.
 */
function ColumnHeaderCapsule({
  controls,
  shown,
}: {
  controls: WorkspaceColumnHeaderControls;
  /** Rail mode: shown; full screen: hidden (the chat's bar returns). */
  shown: boolean;
}) {
  const { colors } = useTheme();
  const surface = useFloatingSurfaceColor();
  const leading = useRef<View>(null);
  const { leading: lead, identity, more } = controls;
  // Menu/Back is the chat's primary control for focus restoration (the
  // drawer returns VoiceOver focus to it): the shown placement claims it.
  useLayoutEffect(() => {
    if (shown) lead.registerFocus?.(() => findNodeHandle(leading.current), true);
  }, [lead.registerFocus, shown]);
  return (
    <RailShortcutsView
      testID="quick-rail-header-motion"
      fadeOpacity={!floatingGlassAvailable()}
      pointerEvents={shown ? "box-none" : "none"}
      accessibilityElementsHidden={!shown}
      importantForAccessibility={shown ? "auto" : "no-hide-descendants"}
    >
      <FloatingCapsule
        testID="quick-rail-header"
        visible={shown}
        glass="rail"
        style={[styles.capsule, styles.group]}
      >
        <FloatingItem
          ref={leading}
          accessibilityLabel={lead.label}
          onPress={lead.onPress}
          disabled={!shown || Boolean(lead.disabled)}
        >
          <RailShortcutIcon>
            {lead.role === "back" ? (
              <ChevronLeft size={24} color={colors.textPrimary} />
            ) : (
              <Menu size={22} color={colors.textPrimary} />
            )}
          </RailShortcutIcon>
        </FloatingItem>
        <FloatingItem
          accessibilityLabel={identity.label}
          onPress={identity.onPress}
          disabled={!shown || Boolean(identity.disabled)}
        >
          <RailShortcutIcon>
            {identity.members ? (
              <AgentAvatarStack
                agents={identity.members}
                size="xs"
                max={2}
                surface={surface}
              />
            ) : identity.agent ? (
              <AgentAvatar agent={identity.agent} size="lg" />
            ) : null}
          </RailShortcutIcon>
        </FloatingItem>
        <FloatingItem
          accessibilityLabel={more.label}
          onPress={more.onPress}
          disabled={!shown || Boolean(more.disabled)}
        >
          <RailShortcutIcon>
            <MoreHorizontal size={23} color={colors.textPrimary} />
          </RailShortcutIcon>
        </FloatingItem>
      </FloatingCapsule>
    </RailShortcutsView>
  );
}

/** A shortcut in the grouped capsule; its icon fades and it is disabled while hidden. */
function RailItem({
  label,
  onPress,
  children,
  visible,
}: {
  label: string;
  onPress: () => void;
  children: ReactNode;
  visible: boolean;
}) {
  return (
    <FloatingItem
      accessibilityLabel={label}
      onPress={onPress}
      disabled={!visible}
    >
      {/* Only the icon fades (never the glass's ancestor, which would break
          it). */}
      <RailShortcutIcon>{children}</RailShortcutIcon>
    </FloatingItem>
  );
}

const styles = StyleSheet.create({
  // The whole column (measured): its empty part passes touches through.
  column: { flex: 1, alignSelf: "stretch", alignItems: "center" },
  // The capsules hug their contents at the top of the column; nothing
  // stretches to the bottom of the screen.
  rail: {
    width: QUICK_RAIL_WIDTH,
    alignItems: "center",
    gap: QUICK_RAIL_SECTION_GAP,
  },
  // Both capsules: the same width and fully rounded ends (one family).
  capsule: {
    width: QUICK_RAIL_TAB_SIZE,
    borderRadius: CAPSULE_RADIUS,
  },
  // The grouped capsule stacks its shortcuts evenly.
  group: {
    alignItems: "center",
    gap: QUICK_RAIL_BUTTON_GAP,
  },
  // Scrolling entries fill the capsule's width.
  scroll: { alignSelf: "stretch" },
  dot: {
    position: "absolute",
    right: -3,
    top: -2,
    width: 10,
    height: 10,
    borderRadius: 5,
    borderWidth: 2,
  },
});
