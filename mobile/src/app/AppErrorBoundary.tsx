import { Component, type ReactNode } from "react";
import { StyleSheet, Text, View } from "react-native";

type Props = { children: ReactNode };
type State = { hasError: boolean; error?: Error };

export class AppErrorBoundary extends Component<Props, State> {
  override state: State = { hasError: false };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  override render() {
    if (this.state.hasError) {
      return (
        <View style={styles.container}>
          <Text style={styles.title}>Something went wrong</Text>
          <Text style={styles.subtitle}>Please restart the app.</Text>
        </View>
      );
    }
    return this.props.children;
  }
}

// Class component can't use the theme hook, and the boundary must render
// even if React tree state is broken — keep the styles inline. Values mirror
// `darkColors` from mobileTheme.ts (kept in sync by hand).
const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#07060e",
    justifyContent: "center",
    alignItems: "center",
    padding: 24,
    gap: 8,
  },
  title: {
    color: "#e8e4f0",
    fontFamily: "SpaceGrotesk_600SemiBold",
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "600",
  },
  subtitle: {
    color: "#7a7490",
    fontFamily: "Manrope_400Regular",
    fontSize: 14,
    lineHeight: 19,
    fontWeight: "400",
  },
});
