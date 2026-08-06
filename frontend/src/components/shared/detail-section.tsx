import { cn } from "@/lib/utils";

/* ── NyxID Detail Section ── */
interface DetailSectionProps {
  readonly title: string;
  readonly children: React.ReactNode;
  /** Trailing control rendered in the title strip (e.g. an edit toggle). */
  readonly action?: React.ReactNode;
  /**
   * Override the section fill. Defaults to `bg-card`, which is correct on the
   * page background; pass `bg-overlay` when the section is nested inside a
   * Card so it still reads as a distinct layer.
   */
  readonly className?: string;
}

export function DetailSection({
  title,
  children,
  action,
  className,
}: DetailSectionProps) {
  return (
    <div
      className={cn(
        "rounded-xl border border-border/50 bg-card overflow-hidden",
        className,
      )}
    >
      <div className="flex items-center justify-between gap-3 border-b border-border/50 px-4 py-2.5">
        <h3 className="text-[13px] font-semibold text-foreground">{title}</h3>
        {action}
      </div>
      <div className="divide-y divide-border/30">{children}</div>
    </div>
  );
}
