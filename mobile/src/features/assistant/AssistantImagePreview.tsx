import { useMemo } from "react";
import {
  Image,
  Modal,
  PanResponder,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { TOUCH_TARGET } from "../../theme/designTokens";

/** A local overlay keeps the transcript and composer mounted while inspecting images. */
export function AssistantImagePreview({
  uri,
  label,
  closeLabel,
  onClose,
  onError,
}: {
  uri?: string;
  label: string;
  closeLabel: string;
  onClose: () => void;
  onError?: () => void;
}) {
  const { colors } = useTheme();
  const pan = useMemo(
    () =>
      PanResponder.create({
        onMoveShouldSetPanResponder: (_, gesture) =>
          gesture.dy > 12 && gesture.dy > Math.abs(gesture.dx),
        onPanResponderRelease: (_, gesture) => {
          if (gesture.dy > 80 || (gesture.dy > 12 && gesture.vy > 0.8))
            onClose();
        },
      }),
    [onClose],
  );
  return (
    <Modal
      visible={!!uri}
      presentationStyle="fullScreen"
      animationType="fade"
      onRequestClose={onClose}
    >
      {/* Native Modal owns another window; measure its insets instead of inheriting a drawer's. */}
      <SafeAreaProvider style={styles.fill}>
        <SafeAreaView
          style={[styles.fill, { backgroundColor: colors.bg }]}
          accessibilityViewIsModal
          onAccessibilityEscape={onClose}
        >
          <View style={[styles.header, { borderColor: colors.border }]}>
            <Label numberOfLines={1} style={styles.title}>
              {label}
            </Label>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={closeLabel}
              onPress={onClose}
              hitSlop={4}
              style={styles.close}
            >
              <X size={22} color={colors.textPrimary} />
            </Pressable>
          </View>
          <View {...pan.panHandlers} style={styles.fill}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Dismiss image preview"
              accessibilityHint="Tap or swipe down to return to the conversation."
              onPress={onClose}
              style={styles.fill}
            >
              {uri ? (
                <Image
                  accessibilityLabel={label}
                  source={{ uri }}
                  resizeMode="contain"
                  style={styles.image}
                  onError={onError}
                />
              ) : null}
            </Pressable>
          </View>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  title: { flex: 1, minWidth: 0 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 16,
    paddingVertical: 8,
    borderBottomWidth: 1,
  },
  close: {
    width: TOUCH_TARGET,
    height: TOUCH_TARGET,
    alignItems: "center",
    justifyContent: "center",
  },
  image: { flex: 1, width: "100%" },
});
