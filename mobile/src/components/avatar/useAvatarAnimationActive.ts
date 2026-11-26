import { useContext, useEffect, useState } from "react";
import { AppState } from "react-native";
import { NavigationContext } from "@react-navigation/native";

/**
 * Animated avatars run only while their screen is focused and the app is in
 * front; otherwise they rest in the mood's static pose. Outside a navigator
 * (sheets, tests) only the app state counts.
 */
export function useAvatarAnimationActive(): boolean {
  const navigation = useContext(NavigationContext);
  const [focused, setFocused] = useState(() => navigation?.isFocused() ?? true);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  useEffect(() => {
    if (!navigation) return;
    setFocused(navigation.isFocused());
    const offFocus = navigation.addListener("focus", () => setFocused(true));
    const offBlur = navigation.addListener("blur", () => setFocused(false));
    return () => {
      offFocus();
      offBlur();
    };
  }, [navigation]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  return focused && foreground;
}
