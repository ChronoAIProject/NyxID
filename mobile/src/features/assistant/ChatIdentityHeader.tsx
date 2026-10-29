import { WORKSPACE_HEADER_ROW } from "../../components/workspace/workspaceHeaderMetrics";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { ChevronDown } from "lucide-react-native";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import type { AgentIdentity } from "../../components/agentAvatarModel";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { formatRelativeTimeFromMs } from "../../lib/time";

/** iOS centers native title views; a shared row anchors identity beside Menu/Back. */
export function ChatHeaderFrame(props: NativeStackHeaderProps) {
  // Navigation invokes `header` as a render callback, inside the navigator.
  // Hooks belong to the child component's own React render.
  return <ChatHeaderContent {...props} />;
}

function ChatHeaderContent({ options, back, route }: NativeStackHeaderProps) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const props = { canGoBack: !!back, tintColor: colors.textPrimary };
  return (
    <View
      testID="workspace-chat-header"
      style={{ paddingTop: insets.top, backgroundColor: colors.bg }}
    >
      <View style={styles.header}>
        {options.headerLeft?.(props)}
        <View style={styles.identitySlot}>
          {typeof options.headerTitle === "function"
            ? options.headerTitle({
                children: options.title ?? route.name,
                tintColor: colors.textPrimary,
              })
            : null}
        </View>
        {options.headerRight?.(props)}
      </View>
    </View>
  );
}
export function chatThreadInfo(
  title?: string | null,
  handle?: string | null,
  lastActivity?: string | null,
  now = Date.now(),
) {
  const cleanHandle = handle?.trim().replace(/^@/, "");
  const name = title?.trim() || (cleanHandle ? `@${cleanHandle}` : "New chat");
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
  disabled,
  onPress,
}: {
  name: string;
  detail: string;
  agent?: AgentIdentity;
  members?: readonly AgentIdentity[];
  status?: "Needs you" | "Running";
  disabled?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Chatting with ${name}, ${detail}${status ? `, ${status}` : ""}. Choose agent`}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={styles.identity}
    >
      {members ? (
        <AgentAvatarStack
          agents={members}
          size="xl"
          max={2}
          surface={colors.bg}
        />
      ) : agent ? (
        <AgentAvatar agent={agent} size="xl" />
      ) : null}
      <View style={styles.labels}>
        <View style={styles.nameLine}>
          <Text
            numberOfLines={1}
            ellipsizeMode="tail"
            allowFontScaling
            maxFontSizeMultiplier={1.35}
            style={[styles.name, { color: colors.textPrimary }]}
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
              {status}
            </Text>
          ) : null}
        </View>
        <Text
          numberOfLines={1}
          ellipsizeMode="tail"
          allowFontScaling
          maxFontSizeMultiplier={1.4}
          style={[styles.detail, { color: colors.textMuted }]}
        >
          {detail}
        </Text>
      </View>
      <ChevronDown size={13} color={colors.textSecondary} />
    </Pressable>
  );
}
const styles = StyleSheet.create({
  header: {
    ...WORKSPACE_HEADER_ROW,
    paddingHorizontal: 8,
    gap: 4,
  },
  identitySlot: { flex: 1, minWidth: 0 },
  identity: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
  },
  labels: { flex: 1, minWidth: 0 },
  nameLine: { flexDirection: "row", alignItems: "center", gap: 6, minWidth: 0 },
  name: {
    fontFamily: fonts.bodySemi,
    fontSize: 16,
    lineHeight: 21,
    flexShrink: 1,
  },
  detail: { fontFamily: fonts.bodyRegular, fontSize: 11, lineHeight: 16 },
  status: { fontFamily: fonts.bodySemi, fontSize: 9, flexShrink: 0 },
});
