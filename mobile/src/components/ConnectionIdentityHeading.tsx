import { StyleSheet, Text, View } from "react-native";
import Svg, { Circle, SvgXml } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import { fonts } from "../theme/designTokens";
import { nyxidIconXml } from "./brand/brandVectors";
import { ServiceIcon } from "./ServiceIcon";

// The web ConnectionArc's eleven dots on an 80×32 viewBox.
const ARC_POINTS = Array.from({ length: 11 }, (_, index) => {
  const t = index / 10;
  return { x: 2 + 76 * t, y: 24 - 4 * 8 * t * (1 - t) };
});

/**
 * NyxID ↔ service identity row with the request title, matching the hosted
 * connector page and the CLI wizard's service step on the web.
 */
export function ConnectionIdentityHeading({
  serviceSlug,
  serviceName,
  iconUrl,
  title,
  subtitle,
  complete = false,
}: {
  serviceSlug: string;
  serviceName: string;
  iconUrl?: string | null;
  title: string;
  subtitle?: string;
  complete?: boolean;
}) {
  const { colors } = useTheme();
  const circle = [
    styles.circle,
    { borderColor: colors.border, backgroundColor: colors.bg },
  ];
  return (
    <View style={styles.root}>
      <View
        accessible
        accessibilityRole="image"
        accessibilityLabel={`NyxID connects to ${serviceName}`}
        style={styles.identities}
      >
        <View style={styles.identity}>
          <View style={circle}>
            <SvgXml xml={nyxidIconXml} width={36} height={36} />
          </View>
          <Text style={[styles.name, { color: colors.textPrimary }]}>
            NyxID
          </Text>
        </View>
        <Svg width={80} height={32} viewBox="0 0 80 32" style={styles.arc}>
          {ARC_POINTS.map(({ x, y }) => (
            <Circle
              key={x}
              cx={x}
              cy={y}
              r={1.1}
              fill={complete ? colors.success : colors.textMuted}
            />
          ))}
        </Svg>
        <View style={styles.identity}>
          <View style={circle}>
            <ServiceIcon
              slug={serviceSlug}
              iconUrl={iconUrl}
              size="xl"
              color={colors.textPrimary}
            />
          </View>
          <Text
            numberOfLines={2}
            style={[styles.name, { color: colors.textPrimary }]}
          >
            {serviceName}
          </Text>
        </View>
      </View>
      <View style={styles.copy}>
        <Text
          accessibilityRole="header"
          style={[styles.title, { color: colors.textPrimary }]}
        >
          {title}
        </Text>
        {subtitle ? (
          <Text style={[styles.subtitle, { color: colors.textSecondary }]}>
            {subtitle}
          </Text>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { gap: 20, alignItems: "center" },
  identities: { flexDirection: "row", alignItems: "flex-start" },
  identity: { width: 88, alignItems: "center", gap: 10 },
  circle: {
    width: 72,
    height: 72,
    borderRadius: 36,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    overflow: "hidden",
  },
  arc: { marginTop: 12 },
  name: { fontFamily: fonts.body, fontSize: 12, textAlign: "center" },
  copy: { gap: 8, alignItems: "center" },
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 20,
    lineHeight: 26,
    textAlign: "center",
  },
  subtitle: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 20,
    textAlign: "center",
  },
});
