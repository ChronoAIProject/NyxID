import { useMemo } from "react";
import { useTheme } from "../../theme/ThemeContext";
import { authSurfaceColors } from "./authAppearance";

export function useAuthTheme() {
  const theme = useTheme();
  const colors = useMemo(
    () => authSurfaceColors(theme.colors, theme.mode),
    [theme.colors, theme.mode],
  );
  return { ...theme, colors };
}
