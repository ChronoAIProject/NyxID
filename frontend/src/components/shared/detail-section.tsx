/* ── VoidPortal Detail Section ── */
interface DetailSectionProps {
  readonly title: string;
  readonly children: React.ReactNode;
}

export function DetailSection({ title, children }: DetailSectionProps) {
  return (
    <div className="rounded-xl border border-border p-4">
      <h3 className="mb-3 font-display text-sm font-semibold">{title}</h3>
      <div className="space-y-2">{children}</div>
    </div>
  );
}
