import { Moon, Sun } from "lucide-react";
import { useThemeStore } from "@/stores/theme-store";
import { useResolvedTheme } from "@/hooks/use-theme";
import { cn } from "@/lib/utils";

/**
 * Light/dark toggle for the dashboard chrome. Shows the icon of the theme it
 * will switch *to*. The first click leaves follow-system and pins an explicit
 * mode (persisted); until then the dashboard tracks the OS preference.
 */
export function ThemeToggle({ className }: { readonly className?: string }) {
  const toggle = useThemeStore((s) => s.toggle);
  const isDark = useResolvedTheme() === "dark";

  return (
    <button
      type="button"
      onClick={toggle}
      aria-label={isDark ? "Switch to light mode" : "Switch to dark mode"}
      title={isDark ? "Switch to light mode" : "Switch to dark mode"}
      className={cn(
        "flex h-8 w-8 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors duration-300 hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-none",
        className,
      )}
    >
      {isDark ? (
        <Sun className="h-[14px] w-[14px]" />
      ) : (
        <Moon className="h-[14px] w-[14px]" />
      )}
    </button>
  );
}
