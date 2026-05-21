import type { ReactNode } from "react";
import {
  Info,
  Lightbulb,
  TriangleAlert,
  ShieldAlert,
  type LucideIcon,
} from "lucide-react";

// Rendered from `:::note` / `:::tip` / `:::warning` / `:::info` / `:::danger`
// directive blocks in docs markdown (see docs-markdown.tsx remarkCallouts).
const NOTE = { Icon: Info, box: "border-nyx-500/30 bg-nyx-500/[0.06]", tint: "text-nyx-secondary-400" };
const STYLES: Record<string, { Icon: LucideIcon; box: string; tint: string }> = {
  note: NOTE,
  info: { Icon: Info, box: "border-sky-500/30 bg-sky-500/[0.06]", tint: "text-sky-400" },
  tip: { Icon: Lightbulb, box: "border-emerald-500/30 bg-emerald-500/[0.06]", tint: "text-emerald-400" },
  warning: { Icon: TriangleAlert, box: "border-amber-500/30 bg-amber-500/[0.06]", tint: "text-amber-400" },
  danger: { Icon: ShieldAlert, box: "border-destructive/30 bg-destructive/[0.06]", tint: "text-destructive" },
};

export function Callout({
  type = "note",
  children,
}: {
  readonly type?: string;
  readonly children: ReactNode;
}) {
  const style = STYLES[type] ?? NOTE;
  const { Icon } = style;
  return (
    <div className={`my-5 flex gap-3 rounded-xl border px-4 py-3 ${style.box}`}>
      <Icon className={`mt-0.5 h-4 w-4 shrink-0 ${style.tint}`} aria-hidden />
      <div className="space-y-2 text-sm leading-relaxed text-muted-foreground [&_a]:text-nyx-secondary-400 [&_code]:text-[0.85em] [&_p]:m-0">
        {children}
      </div>
    </div>
  );
}
