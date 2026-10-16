import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PropsWithChildren,
} from "react";
import { Dimensions, Keyboard, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { keyboardOverlap, type KeyboardFrame } from "./keyboardOverlap";

/** Edge-to-edge Android windows can leave their content underneath the IME.
 * Measure the outer frame, which is unaffected by its own padding. If Android
 * already resized that frame above the IME, no extra space is consumed.
 */
export function AndroidKeyboardAvoidingView({ children }: PropsWithChildren) {
  const insets = useSafeAreaInsets();
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
      setBottom(keyboardOverlap({ y: y + insets.top, height }, frame));
    });
  }, [insets.top]);

  useEffect(() => {
    mounted.current = true;
    keyboard.current = Keyboard.metrics();
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

const styles = StyleSheet.create({ fill: { flex: 1 } });
