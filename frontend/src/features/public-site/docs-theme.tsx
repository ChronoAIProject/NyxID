import { useApplyTheme } from "@/hooks/use-theme";
import { ThemeToggle } from "@/components/dashboard/theme-toggle";
export function PublicDocsTheme() {
  useApplyTheme();
  return <ThemeToggle />;
}
