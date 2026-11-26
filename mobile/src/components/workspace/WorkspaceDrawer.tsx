
import { translateApp } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { WORKSPACE_HEADER_ROW } from "./workspaceHeaderMetrics";
import { useEffect, useState, type Ref } from "react";
import { Text, AppState, Pressable, StyleSheet, View } from "react-native";
import {
  SafeAreaView,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import {
  Bell,
  CalendarClock,
  Monitor,
  Plug,
  Search,
  ScanQrCode,
  Terminal,
  Settings,
  ShieldCheck,
  SquarePen,
  UserPlus,
} from "lucide-react-native";
import { useQuery } from "@tanstack/react-query";
import { LinearGradient } from "expo-linear-gradient";
import { NyxidLogo } from "../brand/NyxidLogo";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import { subscribeToApprovalRefreshSignals } from "../../lib/notifications/approvalRefreshSignal";
import { notificationBellLabel } from "../../features/notifications/notificationPresentation";
import { useNotificationIndicator } from "../../features/notifications/useNotifications";
import { WorkspaceChatList } from "./WorkspaceChatList";
import { WorkspaceRecents } from "./WorkspaceRecents";
import { sidebarBottomSpace, SIDEBAR_BAR_GAP } from "./workspaceSidebarModel";
import { workspaceCloseControlStyle } from "./workspaceCloseControl";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";

export type WorkspaceRoute =
  | "Assistant"
  | "Automations"
  | "Machines"
  | "Activity"
  | "Notifications"
  | "Services"
  | "Plugins"
  | "Agents"
  | "AccountSettings";
const destinations = [
  { route: "Automations",
    get label() {
      return translateApp("Automations");
    }, Icon: CalendarClock, },
  { route: "Machines",
    get label() {
      return translateApp("Machines");
    }, Icon: Monitor, },
  { route: "Plugins",
    get label() {
      return translateApp("Plugins");
    }, Icon: Plug, },
  { route: "Activity",
    get label() {
      return translateApp("Activity");
    }, Icon: ShieldCheck, },
] as const;

type Props = {
  visible: boolean;
  disabled?: boolean;
  currentRoute?: string;
  initialFocusRef?: Ref<View>;
  onNavigate: (route: WorkspaceRoute) => void;
  onHome: () => void;
  onQuickAction: (action: "scanner" | "terminal") => void;
  onBrowse: (route: "WorkspaceSearch" | "WorkspaceRecents") => void;
  currentChat?: WorkspaceChatTarget;
  onOpenChat: (target: WorkspaceChatTarget) => void;
  onCreateAgent: () => void;
  onCreateGroup: () => void;
};
export function WorkspaceDrawer({
  visible,
  disabled = false,
  currentRoute,
  initialFocusRef,
  onNavigate,
  onHome,
  onQuickAction,
  onBrowse,
  currentChat,
  onOpenChat,
  onCreateAgent,
  onCreateGroup,
}: Props) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const [barHeight, setBarHeight] = useState(52);
  const [atEnd, setAtEnd] = useState(false);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  const epoch = getAuthEpoch();
  const active = visible && foreground && isConnected;
  const pending = useQuery({
    queryKey: ["challenges", "sidebar", epoch],
    queryFn: () => mobileApi.getChallenges(1, 1),
    enabled: active,
    refetchInterval: active ? 30000 : false,
    retry: false,
  });
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  useEffect(() => {
    if (!active) return;
    return subscribeToApprovalRefreshSignals(() => {
      if (epoch === getAuthEpoch()) void pending.refetch();
    });
  }, [active, epoch, pending.refetch]);
  const count = pending.data?.total;
  const notifications = useNotificationIndicator(active);
  return (
    <SafeAreaView
      edges={["top", "left"]}
      style={[styles.panel, { backgroundColor: colors.sidebarBase }]}
    >
      <View testID="workspace-sidebar-header-row" style={styles.brandRow}>
        <Pressable
          ref={initialFocusRef}
          accessibilityRole="button"
          disabled={disabled}
          accessibilityLabel={t("NyxID home")}
          accessibilityState={{ disabled }}
          onPress={onHome}
          style={styles.brand}
        >
          <NyxidLogo height={28} />
        </Pressable>
        <Pressable
          accessibilityRole="button"
          disabled={disabled}
          accessibilityLabel={t("Search")}
          accessibilityState={{ disabled }}
          onPress={() => onBrowse("WorkspaceSearch")}
          style={workspaceCloseControlStyle(colors.navActive)}
        >
          <Search
            size={21}
            color={disabled ? colors.textMuted : colors.textPrimary}
          />
        </Pressable>
      </View>
      <WorkspaceChatList
        key={epoch}
        visible={visible}
        disabled={disabled}
        current={currentChat}
        unread={notifications.markers}
        onSelect={onOpenChat}
        onCreateAgent={onCreateAgent}
        onCreateGroup={onCreateGroup}
        scrollProps={{
          style: styles.scroll,
          contentContainerStyle: [
            styles.body,
            { paddingBottom: sidebarBottomSpace(barHeight, insets.bottom) },
          ],
          scrollEventThrottle: 32,
          onScroll: ({ nativeEvent }) => {
            const { contentOffset, layoutMeasurement, contentSize } =
              nativeEvent;
            const end =
              contentOffset.y + layoutMeasurement.height >=
              contentSize.height - 2;
            setAtEnd((old) => (old === end ? old : end));
          },
        }}
        scrollHeader={
          <View>
            <Text
                accessibilityRole="header"
                style={[styles.section, { color: colors.textPrimary }]}
            >
              {t("Quick actions")}
            </Text>
            {(
              [
                { label: translateApp("Scan QR code"), Icon: ScanQrCode, action: "scanner", },
                {
                  label: translateApp("Terminal login code"),
                  Icon: Terminal,
                  action: "terminal",
                },
              ] as const
            ).map(({ label, Icon, action }) => (
              <Pressable
                key={action}
                accessibilityRole="button"
                disabled={disabled}
                accessibilityLabel={t(label)}
                accessibilityState={{ disabled }}
                onPress={() => onQuickAction(action)}
                style={({ pressed }) => [
                  styles.row,
                  {
                    backgroundColor: pressed ? colors.navActive : "transparent",
                  },
                ]}
              >
                <Icon
                  size={20}
                  color={disabled ? colors.textMuted : colors.textSecondary}
                />
                <Text
                  accessibilityLabel={t(label)}
                  style={[
                    styles.rowText,
                    { color: disabled ? colors.textMuted : colors.textPrimary },
                  ]}
                >
                  {t(label)}
                </Text>
              </Pressable>
            ))}
            <Text
                accessibilityRole="header"
                style={[styles.section, { color: colors.textPrimary }]}
            >
              {t("Workspace")}
            </Text>
            {notifications.enabled ? (
              <Pressable
                accessibilityRole="button"
                disabled={disabled}
                accessibilityLabel={notificationBellLabel(
                  notifications.unread,
                  notifications.needsDecision,
                )}
                accessibilityState={{
                  selected: currentRoute === "Notifications",
                  disabled,
                }}
                onPress={() => onNavigate("Notifications")}
                style={({ pressed }) => [
                  styles.row,
                  {
                    backgroundColor:
                      currentRoute === "Notifications" || pressed
                        ? colors.navActive
                        : "transparent",
                  },
                ]}
              >
                <Bell
                  size={20}
                  color={disabled ? colors.textMuted : colors.textSecondary}
                />
                <Text
                  numberOfLines={1}
                  style={[
                    styles.rowText,
                    { color: disabled ? colors.textMuted : colors.textPrimary },
                  ]}
                >
                  {translateApp("Notifications")}
                </Text>
                {notifications.badge ? (
                  <Text
                    testID="sidebar-notifications-count"
                    style={[
                      styles.count,
                      !disabled && notifications.needsDecision
                        ? {
                            color: colors.warningTone.text,
                            backgroundColor: colors.warningTone.bg,
                          }
                        : {
                            color: disabled
                              ? colors.textMuted
                              : colors.textPrimary,
                            backgroundColor: colors.navActive,
                          },
                    ]}
                  >
                    {notifications.badge}
                  </Text>
                ) : null}
              </Pressable>
            ) : null}
            {destinations.map((item) => {
              const route = item.route;
              const selected = route === currentRoute;
              const Icon = item.Icon;
              return (
                <Pressable
                  key={t(item.label)}
                  accessibilityRole="button"
                  disabled={disabled}
                  accessibilityLabel={route === "Activity" && count ? translateApp("{{label}}, {{count}} pending", { label: t(item.label), count: count,}) : t(item.label)}
                  accessibilityState={{ selected, disabled }}
                  onPress={() => onNavigate(route)}
                  style={({ pressed }) => [
                    styles.row,
                    {
                      backgroundColor:
                        selected || pressed ? colors.navActive : "transparent",
                    },
                  ]}
                >
                  <Icon
                    size={20}
                    color={disabled ? colors.textMuted : colors.textSecondary}
                  />
                  <Text
                    numberOfLines={1}
                    style={[
                      styles.rowText,
                      {
                        color: disabled ? colors.textMuted : colors.textPrimary,
                      },
                    ]}
                  >
                    {t(item.label)}
                  </Text>
                  {route === "Activity" && count ? (
                    <Text
                      style={[
                        styles.count,
                        {
                          color: disabled
                            ? colors.textMuted
                            : colors.textPrimary,
                          backgroundColor: colors.navActive,
                        },
                      ]}
                    >
                      {count}
                    </Text>
                  ) : null}
                </Pressable>
              );
            })}
            <Pressable
              accessibilityRole="button"
              disabled={disabled}
              accessibilityLabel={t("Create new agent")}
              accessibilityState={{ disabled }}
              onPress={onCreateAgent}
              style={styles.row}
            >
              <UserPlus
                size={20}
                color={disabled ? colors.textMuted : colors.textSecondary}
              />
              <Text
                accessibilityLabel={t("Create new agent")}
                style={[
                  styles.rowText,
                  { color: disabled ? colors.textMuted : colors.textPrimary },
                ]}
              >
                {t("Create new agent")}
              </Text>
            </Pressable>
          </View>
        }
        scrollFooter={
          <WorkspaceRecents
            visible={visible}
            disabled={disabled}
            unread={notifications.markers}
            onSelect={onOpenChat}
            onSeeAll={() => onBrowse("WorkspaceRecents")}
          />
        }
      />
      <LinearGradient
        testID="sidebar-bottom-fade"
        pointerEvents="none"
        colors={[
          `${colors.sidebarBase}00`,
          colors.sidebarBase,
          colors.sidebarBase,
        ]}
        locations={[
          0,
          atEnd
            ? 0
            : barHeight /
              (sidebarBottomSpace(barHeight, insets.bottom) + barHeight),
          1,
        ]}
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          height:
            sidebarBottomSpace(barHeight, insets.bottom) +
            (atEnd ? 0 : barHeight),
        }}
      />
      <View
        testID="sidebar-bottom-bar"
        onLayout={(event) => {
          const height = Math.ceil(event.nativeEvent.layout.height);
          setBarHeight((old) => (old === height ? old : height));
        }}
        style={[styles.bottomBar, { bottom: insets.bottom + SIDEBAR_BAR_GAP }]}
      >
        <Pressable
          accessibilityRole="button"
          disabled={disabled}
          accessibilityState={{ disabled }}
          accessibilityLabel={t("New chat")}
          accessibilityHint={t("Open NyxBot home")}
          onPress={onHome}
          style={({ pressed }) => [
            styles.chatPill,
            {
              backgroundColor: disabled
                ? colors.navActive
                : pressed
                  ? colors.cardSoft
                  : colors.onPrimary,
            },
          ]}
        >
          <SquarePen
            size={20}
            color={disabled ? colors.textMuted : "#18181B"}
          />
          <Text
            maxFontSizeMultiplier={1.3}
            style={[styles.chatLabel, disabled && { color: colors.textMuted }]}
          >
            {t("Chat")}
          </Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          disabled={disabled}
          accessibilityState={{ disabled }}
          accessibilityLabel={t("Settings")}
          onPress={() => onNavigate("AccountSettings")}
          style={[styles.gear, { backgroundColor: colors.navActive }]}
        >
          <Settings
            size={22}
            color={disabled ? colors.textMuted : colors.textPrimary}
          />
        </Pressable>
      </View>
    </SafeAreaView>
  );
}
const styles = StyleSheet.create({
  panel: { flex: 1 },
  scroll: { flex: 1 },
  brandRow: {
    paddingHorizontal: 20,
    ...WORKSPACE_HEADER_ROW,
    gap: 12,
  },
  brand: { flex: 1, minHeight: 44, justifyContent: "center" },
  body: { paddingHorizontal: 12 },
  section: {
    ...sidebarSectionTitle,
    paddingHorizontal: 12,
    paddingTop: 8,
    paddingBottom: 10,
    flexShrink: 0,
    minWidth: 52,
  },
  row: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 12,
    paddingVertical: 11,
    borderRadius: 8,
  },
  rowText: { flex: 1, fontFamily: fonts.bodySemi, fontSize: 15 },
  count: {
    fontFamily: fonts.bodySemi,
    fontSize: 11,
    borderRadius: 10,
    paddingHorizontal: 7,
    paddingVertical: 2,
  },
  bottomBar: {
    position: "absolute",
    left: 24,
    right: 24,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  chatPill: {
    minHeight: 52,
    borderRadius: 28,
    flexDirection: "row",
    gap: 10,
    paddingVertical: 12,
    paddingHorizontal: 20,
    alignItems: "center",
    borderColor: "#D4D4D8",
    borderWidth: StyleSheet.hairlineWidth,
  },
  chatLabel: { fontFamily: fonts.bodySemi, fontSize: 16, color: "#18181B" },
  gear: {
    width: 52,
    height: 52,
    borderRadius: 26,
    alignItems: "center",
    justifyContent: "center",
  },
});
