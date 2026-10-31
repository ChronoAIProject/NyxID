import type { ReactNode } from "react";
export function SettingsSection({
  title,
  children,
  id,
}: {
  readonly title: string;
  readonly children: ReactNode;
  readonly id?: string;
}) {
  return (
    <section
      id={id}
      aria-label={title}
      className="min-w-0 space-y-3 rounded-xl border border-border/50 bg-card p-4 text-12"
    >
      <h3 className="text-13 font-semibold text-foreground">{title}</h3>
      {children}
    </section>
  );
}
