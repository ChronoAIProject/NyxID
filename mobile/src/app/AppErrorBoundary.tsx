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

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#10101A",
    justifyContent: "center",
    alignItems: "center",
    padding: 24,
  },
  title: {
    color: "#F0EEFF",
    fontSize: 18,
    marginBottom: 8,
  },
  subtitle: {
    color: "#6A6480",
    fontSize: 14,
  },
});
