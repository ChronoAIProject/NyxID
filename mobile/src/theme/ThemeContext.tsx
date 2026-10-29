import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useColorScheme } from "react-native";
import * as SecureStore from "expo-secure-store";
import {
  darkColors,
  darkSheetColors,
  lightColors,
  lightSheetColors,
  type ThemeColors,
} from "./mobileTheme";
import { normalizeLightBubblePalette, type LightBubblePalette } from "./chatAppearance";

export type ThemePreference = "system" | "light" | "dark";
export type ThemeMode = "light" | "dark";

type ThemeContextValue = {
  colors: ThemeColors;
  mode: ThemeMode;
  preference: ThemePreference;
  setPreference: (p: ThemePreference) => void;
  lightBubblePalette: LightBubblePalette;
  setLightBubblePalette: (palette: LightBubblePalette) => void;
};

const STORAGE_KEY = "nyxid_theme_preference";
const BUBBLE_STORAGE_KEY = "nyxid_light_bubble_palette_v1";

const ThemeContext = createContext<ThemeContextValue>({
  colors: lightColors,
  mode: "light",
  preference: "system",
  setPreference: () => {},
  lightBubblePalette: "gray",
  setLightBubblePalette: () => {},
});

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const systemScheme = useColorScheme();
  const [preference, setPreferenceState] = useState<ThemePreference>("system");
  const [loaded, setLoaded] = useState(false);
  const [lightBubblePalette, setLightBubblePaletteState] = useState<LightBubblePalette>("gray");
  const bubbleWrites = useRef(Promise.resolve());

  useEffect(() => {
    let active = true;
    void Promise.all([
      SecureStore.getItemAsync(STORAGE_KEY).catch(() => null),
      SecureStore.getItemAsync(BUBBLE_STORAGE_KEY).catch(() => null),
    ])
      .then(([value, palette]) => {
        if (!active) return;
        if (value === "light" || value === "dark" || value === "system") {
          setPreferenceState(value);
        }
        setLightBubblePaletteState(normalizeLightBubblePalette(palette));
        setLoaded(true);
      })
      .catch(() => { if (active) setLoaded(true); });
    return () => { active = false; };
  }, []);

  const setPreference = useCallback((p: ThemePreference) => {
    setPreferenceState(p);
    SecureStore.setItemAsync(STORAGE_KEY, p).catch(() => {});
  }, []);

  const setLightBubblePalette = useCallback((palette: LightBubblePalette) => {
    const next = normalizeLightBubblePalette(palette);
    setLightBubblePaletteState(next);
    bubbleWrites.current = bubbleWrites.current
      .catch(() => {})
      .then(() => SecureStore.setItemAsync(BUBBLE_STORAGE_KEY, next));
    void bubbleWrites.current.catch(() => {});
  }, []);

  const mode: ThemeMode = useMemo(() => {
    if (preference === "system") return systemScheme === "dark" ? "dark" : "light";
    return preference;
  }, [preference, systemScheme]);

  const colors = mode === "light" ? lightColors : darkColors;

  const value = useMemo<ThemeContextValue>(
    () => ({ colors, mode, preference, setPreference, lightBubblePalette, setLightBubblePalette }),
    [colors, mode, preference, setPreference, lightBubblePalette, setLightBubblePalette],
  );

  if (!loaded) return null;

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  return useContext(ThemeContext);
}

/** Everything rendered inside a pull-up drawer uses the drawer's elevated palette. */
export function SheetThemeProvider({ children }: { children: React.ReactNode }) {
  const parent = useTheme();
  const value = useMemo<ThemeContextValue>(
    () => ({
      ...parent,
      colors: parent.mode === "light" ? lightSheetColors : darkSheetColors,
    }),
    [parent],
  );
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
