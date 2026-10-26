import type { CSSProperties } from "react";

export const chartTick = {
  fill: "var(--analytics-label)",
  fontSize: 11,
  fontFamily: "var(--font-sans)",
};

export const chartTooltipStyle: CSSProperties = {
  background: "var(--color-popover)",
  border: "1px solid var(--color-border)",
  borderRadius: 10,
  padding: "10px 12px",
  boxShadow: "0 8px 24px rgb(0 0 0 / 16%)",
  color: "var(--color-foreground)",
  fontSize: 12,
  maxWidth: "min(420px, calc(100vw - 32px))",
  whiteSpace: "normal",
  overflowWrap: "anywhere",
};

export const chartTooltipTextStyle = {
  color: "var(--color-popover-foreground)",
};
