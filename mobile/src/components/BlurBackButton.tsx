import { BlurView } from "expo-blur";
import { useMemo } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";

type Props = {
  onPress: () => void;
};

export function BlurBackButton({ onPress }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  return (
    <Pressable onPress={onPress} style={styles.wrapper} hitSlop={8}>
      <BlurView intensity={40} tint="dark" style={styles.blur}>
        <Text style={styles.arrow}>{"←"}</Text>
      </BlurView>
    </Pressable>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    wrapper: {
      alignSelf: "flex-start",
      borderRadius: radius.full,
      overflow: "hidden",
      marginBottom: spacing.lg,
    },
    blur: {
      width: 40,
      height: 40,
      borderRadius: radius.full,
      alignItems: "center",
      justifyContent: "center",
      backgroundColor: c.ghostBg,
    },
    arrow: {
      ...typeScale.h2,
      color: c.textPrimary,
      marginTop: -1,
    },
  });
