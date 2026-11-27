import { useApplyTheme } from "@/hooks/use-theme";

/**
 * Applies the app's display settings (text size, density, reduced motion) to the
 * pre-rendered public docs once the public session starts. Colours come from the
 * public day/night choice instead (docs-layout-view), so this renders nothing and
 * the public switch never changes the app's own theme preference.
 */
export function PublicDocsTheme() {
  useApplyTheme();
  return null;
}
