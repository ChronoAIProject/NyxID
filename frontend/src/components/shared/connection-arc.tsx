import { useId } from "react";
import { cn } from "@/lib/utils";

const ARC_POINTS = Array.from({ length: 11 }, (_, index) => {
  const t = index / 10;
  return { x: 2 + 76 * t, y: 24 - 4 * 8 * t * (1 - t) };
});

export function ConnectionArc({ complete = false }: { readonly complete?: boolean }) {
  const arcId = useId();
  const gradientId = `${arcId}-gradient`;
  const maskId = `${arcId}-mask`;

  return (
    <svg
      viewBox="0 0 80 32"
      className={cn(
        "mt-3 h-8 w-20 shrink-0",
        complete ? "text-success/60" : "text-muted-foreground/50",
      )}
      fill="none"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={gradientId}>
          <stop offset="0" stopColor="white" stopOpacity="0" />
          <stop offset="0.35" stopColor="white" stopOpacity="0.25" />
          <stop offset="0.7" stopColor="white" stopOpacity="0.9" />
          <stop offset="1" stopColor="white" stopOpacity="0" />
        </linearGradient>
        <mask id={maskId} maskUnits="userSpaceOnUse" x="0" y="0" width="80" height="32">
          <rect
            width="40"
            height="32"
            fill={`url(#${gradientId})`}
            className="channel-connection-sweep"
          />
        </mask>
      </defs>
      <g fill="currentColor">
        {ARC_POINTS.map(({ x, y }, index) => (
          <circle key={index} cx={x} cy={y} r="1.1" />
        ))}
      </g>
      {!complete && (
        <g fill="currentColor" className="text-foreground/80" mask={`url(#${maskId})`}>
          {ARC_POINTS.map(({ x, y }, index) => (
            <circle key={index} cx={x} cy={y} r="1.1" />
          ))}
        </g>
      )}
    </svg>
  );
}
