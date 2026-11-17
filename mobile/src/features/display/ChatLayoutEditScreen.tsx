import { translateApp } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { Pressable, StyleSheet, View } from "react-native";
import { Minus, Plus } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
import { useChatLayout } from "./ChatLayoutContext";
import {
  chatLayoutBounds,
  chatLayoutTitle,
  defaultChatLayout,
  type ChatLayout,
} from "./chatLayout";
import { ChatPreview } from "./ChatPreview";

type Key = keyof ChatLayout;

const COPY: Record<Key, { step: number; footer: string }> = {
  get columnWidth() {
    return {
    step: 40,
      get footer() {
        return translateApp(
          "How wide conversations can grow on large screens. Saved on this device.",
        );
      },
    };
  },
  get composerMaxHeight() {
    return {
    step: 20,
      get footer() {
        return translateApp(
          "The composer grows with your draft up to this height. Saved on this device.",
        );
      },
    };
  },
};

export { chatLayoutTitle };

/** Settings › Display › one chat layout value with a live preview. */
export function ChatLayoutEditScreen({
  route,
}: SheetScreenProps<"ChatLayoutEdit">) {
  const { t } = useAppLanguage();
  const key = route.params.key;
  const copy = COPY[key];
  const { colors } = useTheme();
  const { layout, update } = useChatLayout();
  const [min, max] = chatLayoutBounds[key];
  const value = layout[key];
  const step = (direction: -1 | 1) => {
    const disabled = direction < 0 ? value <= min : value >= max;
    const Glyph = direction < 0 ? Minus : Plus;
    return (
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t(
          key === "columnWidth"
            ? direction < 0 ? "Decrease chat column width" : "Increase chat column width"
            : direction < 0 ? "Decrease maximum composer height" : "Increase maximum composer height",
        )}
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPress={() => update({ [key]: value + direction * copy.step })}
        style={({ pressed }) => [
          styles.step,
          {
            backgroundColor: pressed ? colors.borderSoft : colors.navActive,
            opacity: disabled ? 0.4 : 1,
          },
        ]}
      >
        <Glyph size={22} color={colors.textPrimary} accessible={false} />
      </Pressable>
    );
  };
  return (
    <SettingsPage>
      <ChatPreview
        composerHeight={key === "composerMaxHeight" ? value : undefined}
        columnRatio={key === "columnWidth" ? value / max : undefined}
      />
      <SettingsGroup inset={false} footer={t(copy.footer)}>
        <View style={styles.stepper}>
          {step(-1)}
          <View style={styles.readout}>
            <Text
              accessibilityLiveRegion="polite"
              style={[styles.value, { color: colors.textPrimary }]}
            >
              {translateApp("{{amount}}pt", { amount: value})}</Text>
            <Text style={[styles.range, { color: colors.textMuted }]}>
              {translateApp("{{min}}–{{max}}pt", { min: min, max: max})}</Text>
          </View>
          {step(1)}
        </View>
        {value !== defaultChatLayout[key] ? (
          <SettingsRow
            centered
            label={t("Reset to {{value}} pt", {
              value: defaultChatLayout[key],
            })}
            onPress={() => update({ [key]: defaultChatLayout[key] })}
          />
        ) : null}
      </SettingsGroup>
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  stepper: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: spacing.xxl,
    paddingVertical: spacing.lg,
  },
  readout: { alignItems: "center", gap: 2 },
  value: { ...typeScale.label, fontSize: 22, lineHeight: 28 },
  range: { ...typeScale.small },
  step: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
});
