import { StyleSheet, Switch, View } from "react-native";
import Svg, { Circle, Path } from "react-native-svg";
import { SunMoon } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  useSettingsStacked,
} from "../../components/settings/SettingsList";
import { capture } from "../../lib/telemetry";
import { useTheme } from "../../theme/ThemeContext";
import { spacing } from "../../theme/designTokens";
import { ChatDisplaySettings } from "../display/ChatDisplaySettings";

/** Settings › Display: chat layout and the app's light/dark appearance. */
export function DisplaySettingsScreen(
  _props: SheetScreenProps<"DisplaySettings">,
) {
  const { colors, mode, preference, setPreference } = useTheme();
  const stacked = useSettingsStacked();
  const isLight =
    preference === "light" || (preference === "system" && mode === "light");
  const isDark =
    preference === "dark" || (preference === "system" && mode === "dark");
  const lightColor = isLight ? colors.primary : colors.textMuted;
  const darkColor = isDark ? colors.primary : colors.textMuted;
  return (
    <SettingsPage>
      <SettingsGroup title="Appearance">
        <View style={styles.segment}>
          <SegmentControl
            accessibilityLabel="Appearance"
            stacked={stacked}
            activeIndex={isLight ? 0 : 1}
            segments={[
              {
                label: "Light",
                accessibilityLabel: "Light appearance",
                icon: (
                  <Svg
                    width={16}
                    height={16}
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke={lightColor}
                    strokeWidth={2}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <Circle cx={12} cy={12} r={5} />
                    <Path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
                  </Svg>
                ),
              },
              {
                label: "Dark",
                accessibilityLabel: "Dark appearance",
                icon: (
                  <Svg
                    width={16}
                    height={16}
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke={darkColor}
                    strokeWidth={2}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <Path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
                  </Svg>
                ),
              },
            ]}
            onPress={(index) => {
              const value = index === 0 ? "light" : "dark";
              capture({
                name: "ui.mobile_preference_toggled",
                props: { name: "theme", value },
              });
              setPreference(value);
            }}
          />
        </View>
        <SettingsRow
          icon={SunMoon}
          tone="primary"
          label="Use system setting"
          accessory={
            <Switch
              accessibilityLabel="Use system setting"
              value={preference === "system"}
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
      <SettingsGroup title="Chat layout">
        <ChatDisplaySettings embedded />
      </SettingsGroup>
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  segment: { padding: spacing.lg },
});
