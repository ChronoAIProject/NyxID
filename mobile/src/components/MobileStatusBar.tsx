import { StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { mobileTheme } from "../theme/mobileTheme";

export function MobileStatusBar() {
  const insets = useSafeAreaInsets();
  return <View style={[styles.wrap, { height: insets.top }]} />;
}

const styles = StyleSheet.create({
  wrap: {
    backgroundColor: mobileTheme.bg,
  },
});
