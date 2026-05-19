import { useMemo, useState } from "react";
import { Pressable, StyleSheet, View, Text } from "react-native";
import Svg, { Circle, Path, Defs, LinearGradient, Stop } from "react-native-svg";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";

type NyxFABProps = {
  onPress?: () => void;
  badgeCount?: number;
};

const FAB_SIZE = 57;
const GLOW_SIZE = 110;

export function NyxFAB({ onPress, badgeCount = 0 }: NyxFABProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const [tapped, setTapped] = useState(false);

  const handlePress = () => {
    setTapped(true);
    onPress?.();
    setTimeout(() => setTapped(false), 500);
  };

  return (
    <View style={styles.wrapper}>

      {/* FAB button */}
      <Pressable style={[styles.fab, tapped && styles.fabTapped]} onPress={handlePress} hitSlop={{ top: 20, bottom: 20, left: 20, right: 20 }}>
        <Svg width={36} height={36} viewBox="0 0 130 130" fill="none">
          <Defs>
            <LinearGradient id="fo" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65">
              <Stop offset="0" stopColor="#A78BFA" />
              <Stop offset="0.5" stopColor="#A78BFA" stopOpacity={0} />
            </LinearGradient>
            <LinearGradient id="fm" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65" gradientTransform="rotate(120 65 65)">
              <Stop offset="0" stopColor="#C4B5FD" />
              <Stop offset="0.5" stopColor="#C4B5FD" stopOpacity={0} />
            </LinearGradient>
            <LinearGradient id="fi" gradientUnits="userSpaceOnUse" x1="10" y1="65" x2="120" y2="65" gradientTransform="rotate(240 65 65)">
              <Stop offset="0" stopColor="#DDD6FE" />
              <Stop offset="0.5" stopColor="#DDD6FE" stopOpacity={0} />
            </LinearGradient>
            <LinearGradient id="fv" gradientUnits="userSpaceOnUse" x1="56" y1="62" x2="86" y2="62" gradientTransform="rotate(160 71 62)">
              <Stop offset="0" stopColor="#C4B5FD" />
              <Stop offset="1" stopColor="#7C3AED" />
            </LinearGradient>
          </Defs>
          <Circle cx={65} cy={65} r={55} fill="none" stroke="url(#fo)" strokeWidth={1} />
          <Circle cx={65} cy={65} r={40} fill="none" stroke="url(#fm)" strokeWidth={1} />
          <Circle cx={65} cy={65} r={25} fill="none" stroke="url(#fi)" strokeWidth={0.8} />
          <Path d="M24 0q6 8 6 20 0 12-6 20-14-4-20-12-4-14-2-24 4-4 22-4z" transform="translate(56 42)" fill="url(#fv)" />
          <Circle cx={31.5} cy={49.5} r={1.5} fill="#C4B5FD" />
          <Circle cx={39} cy={63} r={1} fill="#C4B5FD" opacity={0.5} />
          <Circle cx={25} cy={69} r={1} fill="#C4B5FD" opacity={0.31} />
        </Svg>

        {badgeCount > 0 ? (
          <View style={styles.badge}>
            <Text style={styles.badgeText}>{badgeCount > 9 ? "9+" : badgeCount}</Text>
          </View>
        ) : null}
      </Pressable>
    </View>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    wrapper: {
      width: FAB_SIZE,
      height: FAB_SIZE,
      alignItems: "center",
      justifyContent: "center",
      overflow: "visible",
    },
    fab: {
      width: FAB_SIZE,
      height: FAB_SIZE,
      borderRadius: FAB_SIZE / 2,
      backgroundColor: c.fabBg,
      borderWidth: 1.5,
      borderColor: c.fabBorder,
      alignItems: "center",
      justifyContent: "center",
    },
    fabTapped: {
      borderColor: "#A78BFA",
      borderWidth: 3,
    },
    badge: {
      position: "absolute",
      top: -2,
      right: -2,
      width: 14,
      height: 14,
      borderRadius: 7,
      backgroundColor: c.danger,
      borderWidth: 2,
      borderColor: c.bg,
      alignItems: "center",
      justifyContent: "center",
    },
    badgeText: {
      fontSize: 8,
      fontWeight: "800",
      color: c.onPrimary,
    },
  });
