import type { ReactNode } from "react";

export function MetricBlock({
  label,
  value,
  detail,
}: {
  readonly label: string;
  readonly value: string;
  readonly detail?: ReactNode;
}) {
  return (
    <div className="rounded-lg border border-border/70 bg-overlay px-3 py-3">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className="mt-1 truncate text-[20px] font-semibold leading-tight">
        {value}
      </div>
      {detail && (
        <div className="mt-2 text-[11px] text-muted-foreground">{detail}</div>
      )}
    </div>
  );
}
