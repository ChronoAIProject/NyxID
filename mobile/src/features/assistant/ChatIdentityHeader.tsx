import { Text, Platform } from "react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import {
  FLOATING_HEADER_GAP,
  FLOATING_HEADER_MARGIN,
  WORKSPACE_HEADER_ROW,
} from "../../components/workspace/workspaceHeaderMetrics";
import { useContext, useState } from "react";
import {
  WorkspaceCornerSensorContext,
  WorkspaceQuickRailContext,
} from "../../components/workspace/WorkspaceLayoutContext";
import { Pressable, StyleSheet, View, useWindowDimensions } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { ChevronDown } from "lucide-react-native";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import type { AvatarMood } from "../../lib/avatars/agentAvatars";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { formatRelativeTimeFromMs } from "../../lib/time";
import {
  workspaceStatusCluster,
  workspaceHeaderBand,
  workspaceHeaderRightPadding,
} from "../../components/workspace/workspaceLayout";
import { FloatingButton } from "../../components/FloatingControl";
import { LinearGradient } from "expo-linear-gradient";
import { WorkspaceRailMotionContext } from "../../components/workspace/workspaceFloatingInset";
import {
  HEADER_ACTION_ENTERING,
  HEADER_ACTION_EXITING,
  MotionView,
} from "../../components/workspace/WorkspaceSlabMotion";

/** iOS centers native title views; a shared row anchors identity beside Menu/Back. */
export function ChatHeaderFrame(props: NativeStackHeaderProps) {
  useAppLanguage();
  // Navigation invokes `header` as a render callback, inside the navigator.
  // Hooks belong to the child component's own React render.
  return <ChatHeaderContent {...props} />;
}

function ChatHeaderContent({ options, back, route }: NativeStackHeaderProps) {
  useAppLanguage();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const window = useWindowDimensions();
  const [measuredWidth, setMeasuredWidth] = useState<number>();
  const cornerSensor = useContext(WorkspaceCornerSensorContext);
  const narrow = !cornerSensor &&
    Math.min(window.width, measuredWidth ?? window.width) /
      Math.max(1, window.fontScale ?? 1) < 340;
  // The Duo (folded or unfolded), rail mode: no top bar. The chat's controls
  // (Menu or Back, the agent, More) sit in the status column below the
  // toggle, so the chat starts at the top inset. Full screen: the chat's own
  // bar returns (the same as an iPhone's), fading in, its trailing controls
  // left of the status column and its content right under the row.
  const rail = useContext(WorkspaceQuickRailContext);
  const fullScreen = useContext(WorkspaceRailMotionContext)?.fullScreen ?? false;
  if (rail && !fullScreen)
    return (
      <View
        testID="workspace-chat-header"
        style={{ height: insets.top, backgroundColor: colors.bg }}
      />
    );
  // Like a navigation bar under a status bar: the row sits at the top inset.
  const band = workspaceHeaderBand(
    insets.top,
    cornerSensor,
    workspaceStatusCluster({ platform: Platform.OS, ...window, rightInset: insets.right, }),
    undefined,
    rail,
  );
  const props = { canGoBack: !!back, tintColor: colors.textPrimary };
  const margin = FLOATING_HEADER_MARGIN;
  const identity = typeof options.headerTitle === "function"
    ? options.headerTitle({ children: options.title ?? route.name, tintColor: colors.textPrimary })
    : null;

  return (
    <MotionView
      testID="workspace-chat-header"
      entering={rail ? HEADER_ACTION_ENTERING : undefined}
      exiting={rail ? HEADER_ACTION_EXITING : undefined}
      onLayout={({ nativeEvent: { layout } }) => setMeasuredWidth(layout.width)}
      style={{ ...band, backgroundColor: colors.bg }}
    >
      <View
        testID="workspace-chat-header-row"
        style={{
          ...styles.header,
          gap: FLOATING_HEADER_GAP,
          paddingLeft: margin + insets.left,
          paddingRight: workspaceHeaderRightPadding(insets.right, margin),
        }}
      >
        {options.headerLeft?.(props)}
        <View style={styles.identitySlot}>
          {narrow ? null : identity}
        </View>
        {options.headerRight?.(props)}
      </View>
      {narrow && (
        <View
          testID="workspace-chat-identity-row"
          style={[styles.narrowIdentity, {
            paddingLeft: margin + insets.left,
            paddingRight: workspaceHeaderRightPadding(insets.right, margin),
          }]}
        >
          {identity}
        </View>
      )}
      {rail ? (
        // The Duo's full-screen bar fades into the page: content scrolling
        // under its bottom edge dissolves gradually instead of being cut.
        <LinearGradient
          testID="workspace-chat-header-fade"
          pointerEvents="none"
          colors={[colors.bg, `${colors.bg}00`]}
          style={styles.bottomFade}
        />
      ) : null}
    </MotionView>
  );
}
export function chatThreadInfo(
  title?: string | null,
  handle?: string | null,
  lastActivity?: string | null,
  now = Date.now(),
) {
  const cleanHandle = handle?.trim().replace(/^@/, "");
  const name = title?.trim() || (cleanHandle ? `@${cleanHandle}` : translateApp("New chat"));
  const timestamp = Date.parse(lastActivity ?? "");
  return Number.isFinite(timestamp) && timestamp > 0
    ? `${name} · ${formatRelativeTimeFromMs(timestamp, now).replace(/ ago$/, "")}`
    : name;
}
export function ChatIdentity({
  name,
  detail,
  agent,
  members,
  status,
  mood,
  disabled,
  onPress,
  compact = false,
  floating = false,
}: {
  /** Narrow bars: a smaller fixed avatar and one tail-truncating title line. */
  compact?: boolean;
  /**
   * Narrow bars: avatar, title and chevron share one glass pill like the
   * header buttons; the title is still the only part that truncates.
   */
  floating?: boolean;
  name: string;
  detail: string;
  agent?: AgentIdentity;
  /** The agent's live mood (decorative; `status` carries the state). */
  mood?: AvatarMood;
  members?: readonly AgentIdentity[];
  status?: "Needs you" | "Running";
  disabled?: boolean;
  onPress: () => void;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  // Fixed-size marks (initials never scale): only the title yields width.
  const avatarSize = compact || floating ? "lg" : "xl";
  const control = {
    accessibilityLabel: status
      ? translateApp(
          "Chatting with {{name}}, {{details}}, {{status}}. Choose agent",
          { name: name, details: detail, status: translateApp(status) },
        )
      : translateApp("Chatting with {{name}}, {{details}}. Choose agent", {
          name: name,
          details: detail,
        }),
    disabled,
    onPress,
  };
  const contents = (
    <>
      {members ? (
        <AgentAvatarStack
          agents={members}
          size={avatarSize}
          max={2}
          surface={colors.bg}
        />
      ) : agent ? (
        <AgentAvatar agent={agent} size={avatarSize} mood={mood} animated={!compact && !floating} />
      ) : null}
      <View style={styles.labels}>
        <View style={styles.nameLine}>
          <Text
            numberOfLines={1}
            ellipsizeMode="tail"
            allowFontScaling
            maxFontSizeMultiplier={1.35}
            style={[
              styles.name,
              // A lone title line keeps its natural height so scaled descenders
              // are never clipped; the two-line identity keeps its rhythm.
              compact ? null : styles.nameLineHeight,
              { color: colors.textPrimary },
            ]}
          >
            {name}
          </Text>
          {status ? (
            <Text
              numberOfLines={1}
              maxFontSizeMultiplier={1.25}
              style={[
                styles.status,
                {
                  color:
                    status === "Needs you"
                      ? colors.warningTone.text
                      : colors.textSecondary,
                },
              ]}
            >
              {translateApp(status)}
            </Text>
          ) : null}
        </View>
        {compact ? null : (
          <Text
            numberOfLines={1}
            ellipsizeMode="tail"
            allowFontScaling
            maxFontSizeMultiplier={1.4}
            style={[styles.detail, { color: colors.textMuted }]}
          >
            {detail}
          </Text>
        )}
      </View>
      <ChevronDown size={13} color={colors.textSecondary} />
    </>
  );
  // Floating bars: avatar, title and chevron share the shared glass pill
  // (the same glass button as every floating control).
  return floating ? (
    <FloatingButton
      {...control}
      style={styles.floatingIdentity}
      capsuleStyle={styles.floatingCapsule}
    >
      {contents}
    </FloatingButton>
  ) : (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ disabled: !!disabled }}
      {...control}
      style={styles.identity}
    >
      {contents}
    </Pressable>
  );
}
const styles = StyleSheet.create({
  header: {
    ...WORKSPACE_HEADER_ROW,
    gap: 4,
  },
  identitySlot: { flex: 1, minWidth: 0 },
  narrowIdentity: { minHeight: 44, paddingBottom: 8 },
  // 24pt below the bar, in the page colour fading to clear (drawn over the
  // page: the header sits above the screen's content).
  bottomFade: {
    position: "absolute",
    left: 0,
    right: 0,
    top: "100%",
    height: 24,
  },
  // One control in every layout: the chevron follows the title, and the slot,
  // not the title, absorbs spare width; only the title truncates.
  identity: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
    alignSelf: "flex-start",
    maxWidth: "100%",
  },
  labels: { flexShrink: 1, minWidth: 0 },
  // The 32pt avatar sits concentric in the 44pt pill's rounded end.
  floatingIdentity: {
    flexDirection: "row",
    justifyContent: "flex-start",
    gap: 8,
    minWidth: 0,
    paddingVertical: 3,
    paddingLeft: 6,
    paddingRight: 14,
  },
  // The pill's glass hugs it and yields width (only the title truncates).
  floatingCapsule: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    minWidth: 0,
  },
  nameLine: { flexDirection: "row", alignItems: "center", gap: 6, minWidth: 0 },
  name: {
    fontFamily: fonts.bodySemi,
    fontSize: 16,
    flexShrink: 1,
  },
  nameLineHeight: { lineHeight: 21 },
  detail: { fontFamily: fonts.bodyRegular, fontSize: 11, lineHeight: 16 },
  status: { fontFamily: fonts.bodySemi, fontSize: 9, flexShrink: 0 },
});
