import {
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { SvgXml } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import { fonts } from "../theme/designTokens";
import { nyxidIconXml } from "./brand/brandVectors";
import {
  AGENT_AVATAR_SIZES,
  AGENT_AVATAR_TINTS,
  agentInitials,
  agentTintIndex,
  type AgentIdentity,
} from "./agentAvatarModel";

type Size = keyof typeof AGENT_AVATAR_SIZES;

/** Names belong to the containing row; the identity mark is decorative by default. */
export function AgentAvatar({
  agent,
  size = "md",
  label,
  style,
}: {
  agent: AgentIdentity;
  size?: Size;
  label?: string;
  style?: StyleProp<ViewStyle>;
}) {
  const { mode } = useTheme();
  const dimensions = AGENT_AVATAR_SIZES[size];
  const tint =
    agent.kind === "nyxbot"
      ? {
          border: "rgba(90,42,241,0.30)",
          fill: "rgba(90,42,241,0.10)",
          text: "#5A2AF1",
        }
      : AGENT_AVATAR_TINTS[agentTintIndex(agent.id)]![mode];
  return (
    <View
      accessible={!!label}
      accessibilityRole={label ? "image" : undefined}
      accessibilityLabel={label}
      accessibilityElementsHidden={!label}
      importantForAccessibility={label ? "yes" : "no-hide-descendants"}
      style={[
        styles.circle,
        {
          width: dimensions.circle,
          height: dimensions.circle,
          borderRadius: dimensions.circle / 2,
          borderColor: tint.border,
          backgroundColor: tint.fill,
        },
        style,
      ]}
    >
      {agent.kind === "nyxbot" ? (
        <SvgXml xml={nyxidIconXml} width="70%" height="70%" />
      ) : (
        <Text
          allowFontScaling={false}
          style={{
            fontFamily: fonts.bodySemi,
            fontSize: dimensions.font,
            letterSpacing: size === "xs" ? -0.36 : 0,
            color: tint.text,
          }}
        >
          {agentInitials(agent.display_name?.trim() || agent.name)}
        </Text>
      )}
    </View>
  );
}

export function AgentAvatarStack({
  agents,
  max = 3,
  size = "sm",
  surface,
}: {
  agents: readonly AgentIdentity[];
  max?: number;
  size?: Size;
  surface?: string;
}) {
  const { colors, mode } = useTheme();
  const shown = agents.slice(0, Math.max(0, max));
  const hidden = agents.length - shown.length;
  const dimensions = AGENT_AVATAR_SIZES[size];
  const ring = {
    borderWidth: 2,
    borderRadius: (dimensions.circle + 4) / 2,
    borderColor: surface ?? colors.card,
  };
  return (
    <View
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={styles.stack}
    >
      {shown.map((agent, index) => (
        <View key={agent.id} style={[ring, { marginLeft: index ? -6 : 0 }]}>
          <AgentAvatar agent={agent} size={size} />
        </View>
      ))}
      {hidden > 0 ? (
        <View style={[ring, { marginLeft: shown.length ? -6 : 0 }]}>
          <View
            style={[
              styles.circle,
              {
                width: dimensions.circle,
                height: dimensions.circle,
                borderRadius: dimensions.circle / 2,
                backgroundColor: mode === "dark" ? "#2C2C30" : "#F4F4F5",
                borderColor: colors.border,
              },
            ]}
          >
            <Text
              allowFontScaling={false}
              style={{
                color: colors.textSecondary,
                fontFamily: fonts.bodySemi,
                fontSize: dimensions.font,
              }}
            >
              +{hidden}
            </Text>
          </View>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  circle: {
    flexShrink: 0,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    overflow: "hidden",
  },
  stack: { flexDirection: "row", alignItems: "center", flexShrink: 0 },
});
