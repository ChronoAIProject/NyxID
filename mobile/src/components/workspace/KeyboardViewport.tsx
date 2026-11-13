import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PropsWithChildren,
} from "react";
import { Dimensions, Keyboard, Platform, StyleSheet, View } from "react-native";
import {
  useSafeAreaFrame,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import {
  keyboardInViewportWindow,
  keyboardOverlap,
  type KeyboardFrame,
} from "./keyboardOverlap";

/** Measure the page itself, including sheet/header placement. Padding only the
 * actual overlap avoids a second keyboard reservation after native resize.
 */
export function KeyboardViewport({ children }: PropsWithChildren) {
  const insets = useSafeAreaInsets();
  const safeAreaFrame = useSafeAreaFrame();
  const view = useRef<View>(null);
  const keyboard = useRef<KeyboardFrame | undefined>(Keyboard.metrics());
  const measurement = useRef(0);
  const mounted = useRef(false);
  const [bottom, setBottom] = useState(0);

  const update = useCallback(() => {
    const revision = ++measurement.current;
    const frame = keyboard.current;
    if (!frame) {
      setBottom(0);
      return;
    }
    view.current?.measureInWindow((_x, y, _width, height) => {
      if (!mounted.current || revision !== measurement.current) return;
      // Android Fabric measures from the visible window's top, while IME
      // screenY uses screen coordinates. Restore the current top inset first.
      setBottom(
        keyboardOverlap(
          { y: y + (Platform.OS === "android" ? insets.top : 0), height },
          Platform.OS === "ios"
            ? keyboardInViewportWindow(frame, safeAreaFrame)
            : frame,
        ),
      );
    });
  }, [insets.top, safeAreaFrame.y, safeAreaFrame.height]);

  useEffect(() => {
    mounted.current = true;
    keyboard.current = Keyboard.metrics();
    const changing =
      Platform.OS === "ios"
        ? Keyboard.addListener("keyboardWillChangeFrame", (event) => {
            Keyboard.scheduleLayoutAnimation(event);
            keyboard.current = event.endCoordinates;
            update();
          })
        : undefined;
    const shown = Keyboard.addListener("keyboardDidShow", (event) => {
      keyboard.current = event.endCoordinates;
      update();
    });
    const hidden = Keyboard.addListener("keyboardDidHide", () => {
      keyboard.current = undefined;
      update();
    });
    const resized = Dimensions.addEventListener("change", () => {
      keyboard.current = Keyboard.metrics();
      update();
    });
    update();
    return () => {
      mounted.current = false;
      measurement.current++;
      changing?.remove();
      shown.remove();
      hidden.remove();
      resized.remove();
    };
  }, [update]);

  return (
    <View
      ref={view}
      collapsable={false}
      onLayout={update}
      style={[styles.fill, { paddingBottom: bottom }]}
    >
      {children}
    </View>
  );
}

const styles = StyleSheet.create({ fill: { flex: 1, minHeight: 0 } });
