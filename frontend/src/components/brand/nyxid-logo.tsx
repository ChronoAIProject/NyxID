import { useResolvedTheme } from "@/hooks/use-theme";

/**
 * Brand mark + "NyxID" wordmark from the brand guide
 * (`nyxid-brand-guide/public/logos/coloured_logo.svg`). Picks the dark-text
 * variant on light surfaces, the white-text variant on dark surfaces.
 * For icon-only treatments (no wordmark) use {@link NyxidIcon} instead.
 */
export function NyxidLogo({
  className = "h-7 w-auto",
}: {
  readonly className?: string;
}) {
  const theme = useResolvedTheme();
  const src =
    theme === "light"
      ? "/nyxid-coloured-logo-dark.svg"
      : "/nyxid-coloured-logo.svg";
  return <img src={src} alt="NyxID" className={className} />;
}
