import { StyleSheet, Text, View } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { typeScale } from "../theme/designTokens";

export type ToastKind = "success" | "error" | "info";

export type ToastState = {
  message: string;
  kind: ToastKind;
};

type Props = {
  toast: ToastState | null;
  bottom?: number;
};

export function ToastOverlay({ toast, bottom = 108 }: Props) {
  if (!toast) return null;

  return (
    <View style={[styles.wrap, { bottom }]} pointerEvents="none">
      <View
        style={[
          styles.toast,
          toast.kind === "success" ? styles.toastSuccess : null,
          toast.kind === "error" ? styles.toastError : null,
          toast.kind === "info" ? styles.toastInfo : null,
        ]}
      >
        <Text style={styles.text}>{toast.message}</Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    position: "absolute",
    left: 0,
    right: 0,
    alignItems: "center",
    paddingHorizontal: 24,
  },
  toast: {
    width: "100%",
    borderRadius: 12,
    borderWidth: 1,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  toastSuccess: {
    backgroundColor: "#0F2A1F",
    borderColor: "#1E6D4B",
  },
  toastError: {
    backgroundColor: "#2A1115",
    borderColor: "#8B2A35",
  },
  toastInfo: {
    backgroundColor: "#131A2C",
    borderColor: "#304A8A",
  },
  text: {
    color: mobileTheme.textPrimary,
    ...typeScale.caption,
  },
});
