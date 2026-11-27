import { requireNativeView, requireOptionalNativeModule } from "expo";
import { Platform, View, type ViewProps } from "react-native";

type NativeProps = ViewProps & { blocked: boolean };
const NativeBoundary =
  Platform.OS === "android" && requireOptionalNativeModule("NyxFocusBoundary")
    ? requireNativeView<NativeProps>("NyxFocusBoundary")
    : null;

export function WorkspaceKeyboardFocusBoundary({
  blocked,
  ...props
}: NativeProps) {
  // Expo Go and iOS use the ordinary component tree.
  if (!NativeBoundary) return <View {...props} />;
  return <NativeBoundary blocked={blocked} {...props} />;
}
