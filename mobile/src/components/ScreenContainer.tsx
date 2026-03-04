import { PropsWithChildren } from "react";
import { SafeAreaView, StyleSheet, View } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { spacing } from "../theme/designTokens";

export function ScreenContainer({ children }: PropsWithChildren) {
  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.content}>{children}</View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: mobileTheme.bg },
  content: { flex: 1, paddingHorizontal: spacing.xxl, paddingBottom: spacing.xxxl },
});
