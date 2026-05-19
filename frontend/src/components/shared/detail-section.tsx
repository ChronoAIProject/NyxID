/* ── NyxID Detail Section ── */
interface DetailSectionProps {
  readonly title: string;
  readonly children: React.ReactNode;
}

export function DetailSection({ title, children }: DetailSectionProps) {
  return (
    <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
      <div className="border-b border-border/50 px-4 py-2.5">
        <h3 className="text-[13px] font-semibold text-foreground">{title}</h3>
      </div>
      <div className="divide-y divide-border/30">{children}</div>
    </div>
  );
}
