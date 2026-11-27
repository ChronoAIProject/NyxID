import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PropsWithChildren,
} from "react";
import {
  AppState,
  Dimensions,
  Keyboard,
  Platform,
  StyleSheet,
  View,
} from "react-native";
import { requireOptionalNativeModule } from "expo";
import { observeKeyboardFrames } from "./keyboardFrameEvents";
import {
  useSafeAreaFrame,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import { useWorkspaceSheet } from "../../app/WorkspaceHeader";
import { keyboardOverlap, type KeyboardFrame } from "./keyboardOverlap";

const windowInsets = Platform.OS === "android"
  ? requireOptionalNativeModule<{ contentBottom(): number | null }>("NyxWindowInsets")
  : null;

export type KeyboardViewportFrame = { y: number; height: number };

/** Measure the page itself, including sheet/header placement. Padding only the
 * actual overlap avoids a second keyboard reservation after native resize.
 */
export function KeyboardViewport({
  children,
  onViewportChange,
}: PropsWithChildren<{
  /** Visible bounds in the same window coordinates as measureInWindow. */
  onViewportChange?: (frame: KeyboardViewportFrame | undefined) => void;
}>) {
  const insets = useSafeAreaInsets();
  const providerFrame = useSafeAreaFrame();
  const inSheet = Boolean(useWorkspaceSheet());
  const awaitingSheetLayout = useRef(false);
  const view = useRef<View>(null);
  const keyboard = useRef<KeyboardFrame | undefined>(Keyboard.metrics());
  const measurement = useRef(0);
  const mounted = useRef(false);
  const [bottom, setBottom] = useState(0);
  const notify = useRef(onViewportChange);
  notify.current = onViewportChange;

  const update = useCallback(() => {
    const revision = ++measurement.current;
    const frame = keyboard.current;
    if (!frame) {
      setBottom(0);
      notify.current?.(undefined);
      return;
    }
    view.current?.measureInWindow((_x, y, _width, height) => {
      if (!mounted.current || revision !== measurement.current) return;
      // Android Fabric measures from the visible window's top, while IME
      // screenY uses screen coordinates. Restore the current top inset first.
      const overlap = keyboardOverlap(
        { y: y + (Platform.OS === "android" ? insets.top : 0), height },
        // iOS keyboard events already use the same window as measureInWindow.
        // A nested provider can describe an already-lifted sheet, not the window.
        frame,
        Platform.OS,
      );
      // The sheet's KeyboardAvoidingView lifts after will-change. Do not
      // reserve or reveal against its old position before native layout settles.
      if (awaitingSheetLayout.current && overlap > 0) return;
      setBottom(overlap);
      notify.current?.(
        frame.height > 0 ? { y, height: height - overlap } : undefined,
      );
    });
  }, [insets.top]);

  useEffect(() => {
    mounted.current = true;
    const stop = observeKeyboardFrames({
      platform: Platform.OS,
      keyboard: Keyboard,
      appState: AppState,
      dimensions: Dimensions,
      // Android IME frames use screen coordinates even after adjustResize
      // shortens Dimensions.window. Validate against the matching screen.
      windowHeight: () =>
        Dimensions.get(Platform.OS === "android" ? "screen" : "window").height,
      androidContentBottom: windowInsets ? () => windowInsets.contentBottom() : undefined,
      afterLayout: requestAnimationFrame,
      cancelLayout: cancelAnimationFrame,
      onFrame: (frame, event) => {
        awaitingSheetLayout.current =
          Platform.OS === "ios" && inSheet && !!event && !!frame?.height;
        if (event && !inSheet) Keyboard.scheduleLayoutAnimation(event);
        keyboard.current = frame;
        update();
      },
      onSettled: () => {
        awaitingSheetLayout.current = false;
        if (Platform.OS === "ios") update();
      },
    });
    return () => {
      mounted.current = false;
      measurement.current++;
      stop();
    };
  }, [update, inSheet]);

  // A capped sheet can move without this view's onLayout firing. Its nearest
  // provider is only a layout signal; overlap always uses window coordinates.
  useEffect(() => {
    awaitingSheetLayout.current = false;
    update();
  }, [
    providerFrame.x,
    providerFrame.y,
    providerFrame.width,
    providerFrame.height,
    update,
  ]);

  return (
    <View
      ref={view}
      collapsable={false}
      onLayout={() => {
        awaitingSheetLayout.current = false;
        update();
      }}
      style={[styles.fill, { paddingBottom: bottom }]}
    >
      {children}
    </View>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1, minHeight: 0 } });
