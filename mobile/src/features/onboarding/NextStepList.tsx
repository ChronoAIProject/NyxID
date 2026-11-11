import { useState } from "react";
import { NextStepConnect } from "./NextStepConnect";
import { Feather } from "@expo/vector-icons";
import { Pressable, StyleSheet, View } from "react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { featherIcon, type NextStep } from "./onboardingContracts";

/** Connection steps open hosted setup first; other steps start the chat. */
export function NextStepList({
  steps,
  onStart,
  disabled,
}: {
  steps: readonly NextStep[];
  onStart: (step: NextStep) => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  const [connecting, setConnecting] = useState<NextStep>();
  // OAuth completion can remove this recommendation during a Home refresh.
  // Retain the selected row until its hosted setup delivers the follow-up.
  const displayed = connecting && !steps.some((step) => step.id === connecting.id)
    ? [connecting, ...steps] : steps;
  return (
    <View style={styles.list}>
      {displayed.map((step) => (
        <View
          key={step.id}
          style={[styles.card, { backgroundColor: colors.card, borderColor: colors.border }]}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={step.title}
            accessibilityHint={step.description}
            disabled={disabled}
            onPress={() => step.connect ? setConnecting(step) : onStart(step)}
            style={({ pressed }) => [
              styles.main,
              { opacity: disabled ? 0.6 : pressed ? 0.7 : 1 },
            ]}
          >
            <View
              style={[
                styles.icon,
                { backgroundColor: colors.cardSoft, borderColor: colors.border },
              ]}
            >
              <Feather
                name={featherIcon(step.icon) as keyof typeof Feather.glyphMap}
                size={16}
                color={colors.textPrimary}
              />
            </View>
            <View style={styles.text}>
              <Label style={styles.title}>{step.title}</Label>
              <Label muted style={styles.description}>
                {step.description}
              </Label>
            </View>
            <Label muted style={styles.arrow}>
              →
            </Label>
          </Pressable>
          {connecting?.id === step.id ? <NextStepConnect key={step.id} step={step} onStart={onStart} onClose={() => setConnecting(undefined)} /> : null}
        </View>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  list: { gap: 8 },
  card: {
    borderWidth: 1,
    borderRadius: 12,
  },
  main: {
    alignSelf: "stretch",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    padding: 12,
  },
  icon: {
    width: 36,
    height: 36,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  text: { flex: 1, gap: 2 },
  title: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 19 },
  description: { fontSize: 12, lineHeight: 17 },
  arrow: { fontSize: 14, lineHeight: 19 },
});
