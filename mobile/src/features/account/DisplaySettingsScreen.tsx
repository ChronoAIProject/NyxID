import { Pressable, StyleSheet, Switch, View } from "react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
import { ChatDisplaySettings } from "../display/ChatDisplaySettings";
import { ChatPreview } from "../display/ChatPreview";

/**
 * Settings › Display, in the shape of iOS Display & Brightness: a live preview,
 * Light/Dark tiles with a radio under each, then the per-chat choices.
 */
export function DisplaySettingsScreen(
  _props: SheetScreenProps<"DisplaySettings">,
) {
  const { colors, mode, preference, setPreference } = useTheme();
  const automatic = preference === "system";
  const current = mode === "dark" ? "dark" : "light";
  const choose = (value: "light" | "dark") => {
    capture({
      name: "ui.mobile_preference_toggled",
      props: { name: "theme", value },
    });
    setPreference(value);
  };
  return (
    <SettingsPage>
      <ChatPreview />
      <SettingsGroup title="Appearance" inset={false}>
        <View accessibilityRole="radiogroup" style={styles.tiles}>
          {(["light", "dark"] as const).map((value) => {
            const selected = current === value;
            return (
              <Pressable
                key={value}
                accessibilityRole="radio"
                accessibilityLabel={`${value === "light" ? "Light" : "Dark"} appearance`}
                accessibilityState={{ checked: selected, selected }}
                onPress={() => choose(value)}
                style={styles.tile}
              >
                <View
                  style={[
                    styles.thumb,
                    {
                      borderColor: selected ? colors.primary : "transparent",
                    },
                  ]}
                >
                  <ChatPreview mode={value} compact />
                </View>
                <Text style={[styles.tileLabel, { color: colors.textPrimary }]}>
                  {value === "light" ? "Light" : "Dark"}
                </Text>
                <View
                  style={[
                    styles.radio,
                    {
                      borderColor: selected ? colors.primary : colors.controlBorder,
                      backgroundColor: selected ? colors.primary : "transparent",
                    },
                  ]}
                >
                  {selected ? (
                    <View
                      style={[styles.radioDot, { backgroundColor: colors.onPrimary }]}
                    />
                  ) : null}
                </View>
              </Pressable>
            );
          })}
        </View>
        <SettingsRow
          label="Automatic"
          detail={automatic ? "Follows your iPhone's appearance" : undefined}
          accessory={
            <Switch
              accessibilityLabel="Automatic appearance"
              value={automatic}
              onValueChange={(on) => {
                capture({
                  name: "ui.mobile_preference_toggled",
                  props: { name: "theme_system", value: on },
                });
                setPreference(on ? "system" : mode);
              }}
              trackColor={{ false: colors.borderSoft, true: colors.success }}
            />
          }
        />
      </SettingsGroup>
      <ChatDisplaySettings />
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  tiles: {
    flexDirection: "row",
    justifyContent: "center",
    gap: spacing.huge,
    paddingVertical: spacing.xxl,
    paddingHorizontal: spacing.xxl,
  },
  tile: { flex: 1, maxWidth: 140, alignItems: "center", gap: spacing.sm },
  thumb: { width: "100%", borderRadius: 14, borderWidth: 2, padding: 2 },
  tileLabel: { ...typeScale.label },
  radio: {
    width: 22,
    height: 22,
    borderRadius: 11,
    borderWidth: 1.5,
    alignItems: "center",
    justifyContent: "center",
  },
  radioDot: { width: 8, height: 8, borderRadius: 4 },
});
