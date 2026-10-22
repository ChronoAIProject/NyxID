import type { ComponentProps } from "react";
import { useAuthStore } from "@/stores/auth-store";
import { useApplyTheme } from "@/hooks/use-theme";
import { ThemeToggle } from "@/components/dashboard/theme-toggle";
import { DocsLayout as View } from "./docs-layout-view";
export { SURFACE_ICONS } from "./docs-layout-view";
export type { TocItem } from "./docs-layout-view";
export function DocsLayout(props: ComponentProps<typeof View>) {
  useApplyTheme();
  const isAuthed = useAuthStore((s) => s.isAuthenticated);
  return <View {...props} isAuthed={isAuthed} themeToggle={<ThemeToggle />} />;
}
