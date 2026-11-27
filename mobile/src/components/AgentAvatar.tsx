import type { SharedValue } from "react-native-reanimated";
import {
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import Svg, { Circle, Path, SvgXml } from "react-native-svg";
import { useMemo } from "react";
import { processState } from "../../../shared/agent-avatars/model";
import {
  avatarBadge,
  AVATAR_BADGE_DISC,
  AVATAR_BADGE_VIEW_BOX,
} from "../../../shared/agent-avatars/presentation";
import { useTheme } from "../theme/ThemeContext";
import { fonts } from "../theme/designTokens";
import { useAgentAvatarsEnabled } from "../lib/avatars/agentAvatarsFlag";
import { resolveAvatar, type AvatarMood } from "../lib/avatars/agentAvatars";
import { nyxidIconXml } from "./brand/brandVectors";
import { AvatarAnimated, AvatarStill } from "./avatar/AvatarArt";
import {
  AGENT_AVATAR_SIZES,
  AGENT_AVATAR_TINTS,
  agentInitials,
  agentTintIndex,
  SMALL_AVATAR_PIXELS,
  type AgentIdentity,
} from "./agentAvatarModel";

type Size = keyof typeof AGENT_AVATAR_SIZES;
const BADGE_PIXELS: Record<Size, number> = {
  xs: 12,
  sm: 12,
  md: 12,
  lg: 14,
  xl: 16,
};

/**
 * An agent's avatar. With `assistant:avatars` on: the agent's face (stored
 * or derived), a still at every size unless `animated`, which also needs
 * 24pt or more. Off: NyxBot is the product mark, a specialist its tinted
 * initials, exactly as before. Names belong to the containing row; the
 * avatar is decorative unless `label` is set, and its mood is never
 * announced.
 */
export function AgentAvatar({
  agent,
  size = "md",
  label,
  style,
  mood,
  animated = false,
  pixels,
  eventKey,
  audioLevel,
  animationVisible = true,
}: {
  agent: AgentIdentity;
  size?: Size;
  label?: string;
  style?: StyleProp<ViewStyle>;
  /** The live mood; stills show its static pose, animated views its motion. */
  mood?: AvatarMood;
  /** Only the chat header, the member replying in a group, the agent
   * details screen and the editor preview animate. */
  animated?: boolean;
  /** A box larger than the named sizes (editor preview); face only. */
  pixels?: number;
  eventKey?: string;
  audioLevel?: SharedValue<number>;
  animationVisible?: boolean;
}) {
  const enabled = useAgentAvatarsEnabled();
  if (enabled) {
    return (
      <FaceAvatar
        agent={agent}
        size={size}
        label={label}
        style={style}
        mood={mood}
        animated={animated}
        pixels={pixels}
        eventKey={eventKey}
        audioLevel={audioLevel}
        animationVisible={animationVisible}
      />
    );
  }
  return (
    <LegacyAgentAvatar agent={agent} size={size} label={label} style={style} />
  );
}

function FaceAvatar({
  agent,
  size,
  label,
  style,
  mood,
  animated,
  pixels,
  eventKey,
  audioLevel,
  animationVisible = true,
}: {
  agent: AgentIdentity;
  size: Size;
  label?: string;
  style?: StyleProp<ViewStyle>;
  mood?: AvatarMood;
  animated: boolean;
  pixels?: number;
  eventKey?: string;
  audioLevel?: SharedValue<number>;
  animationVisible?: boolean;
}) {
  const { colors, mode } = useTheme();
  const config = useMemo(
    () =>
      resolveAvatar({
        id: agent.id,
        kind: agent.kind,
        avatar: agent.avatar ?? null,
      }),
    [agent.id, agent.kind, agent.avatar],
  );
  const box = pixels ?? AGENT_AVATAR_SIZES[size].circle;
  // Below 24pt: the simplified still, never animated.
  const small = box < SMALL_AVATAR_PIXELS;
  const state = processState(mood);
  const badge = pixels === undefined ? avatarBadge(state) : null;
  const glyphColors = {
    primary: colors.primaryTone.text,
    warning: colors.warningTone.text,
    success: colors.successTone.text,
    neutral: colors.textPrimary,
  };
  return (
    <View
      accessible={!!label}
      accessibilityRole={label ? "image" : undefined}
      accessibilityLabel={label}
      accessibilityElementsHidden={!label}
      importantForAccessibility={label ? "yes" : "no-hide-descendants"}
      style={[styles.face, { width: box, height: box }, style]}
    >
      {animated && !small ? (
        <AvatarAnimated
          agentId={agent.id}
          eventKey={eventKey}
          audioLevel={audioLevel}
          visible={animationVisible}
          kind={agent.kind}
          config={config}
          mood={mood ?? "idle"}
          theme={mode}
          pixels={box}
        />
      ) : (
        <AvatarStill
          kind={agent.kind}
          config={config}
          mood={mood}
          theme={mode}
          small={small}
          pixels={box}
        />
      )}
      {badge ? (
        <View
          testID={`avatar-activity-${state}`}
          accessible={false}
          accessibilityElementsHidden
          importantForAccessibility="no"
          pointerEvents="none"
          style={styles.badge}
        >
          <Svg
            width={BADGE_PIXELS[size]}
            height={BADGE_PIXELS[size]}
            viewBox={AVATAR_BADGE_VIEW_BOX}
            accessible={false}
          >
            <Circle {...AVATAR_BADGE_DISC} fill={colors.bg} />
            <Circle cx={12} cy={12} r={11.5} stroke={colors.border} fill="none" />
            <Path
              d={badge.glyph.d}
              fill={badge.glyph.strokeWidth ? "none" : glyphColors[badge.tone]}
              stroke={badge.glyph.strokeWidth ? glyphColors[badge.tone] : undefined}
              strokeWidth={badge.glyph.strokeWidth}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </Svg>
        </View>
      ) : null}
    </View>
  );
}

/** Today's avatar, kept unchanged while `assistant:avatars` is off. */
function LegacyAgentAvatar({
  agent,
  size,
  label,
  style,
}: {
  agent: AgentIdentity;
  size: Size;
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
  const faces = useAgentAvatarsEnabled();
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
          {faces ? (
            // Blobs are not round: a muted disc behind each keeps the
            // cutout. It must not clip: headwear rises above the disc.
            <View
              style={[
                styles.disc,
                {
                  width: dimensions.circle,
                  height: dimensions.circle,
                  borderRadius: dimensions.circle / 2,
                  borderColor: "transparent",
                  backgroundColor: mode === "dark" ? "#2C2C30" : "#F4F4F5",
                },
              ]}
            >
              <AgentAvatar
                agent={agent}
                size={size}
                pixels={Math.round(dimensions.circle * 0.86)}
              />
            </View>
          ) : (
            <AgentAvatar agent={agent} size={size} />
          )}
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
  face: { flexShrink: 0, overflow: "visible" },
  badge: { position: "absolute", bottom: -2, right: -2 },
  disc: {
    flexShrink: 0,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    overflow: "visible",
  },
  stack: { flexDirection: "row", alignItems: "center", flexShrink: 0 },
});
