import type { ArticleStatus } from "../types";

const TONE: Record<
  ArticleStatus,
  { label: string; classes: string; dot: string }
> = {
  draft: {
    label: "Draft",
    classes: "border-warning/40 bg-warning/10 text-warning",
    dot: "bg-warning",
  },
  in_review: {
    label: "In review",
    classes: "border-info/40 bg-info/10 text-info",
    dot: "bg-info",
  },
  published: {
    label: "Published",
    classes: "border-success/40 bg-success/10 text-success",
    dot: "bg-success",
  },
  archived: {
    label: "Archived",
    classes: "border-landing-border-subtle bg-landing-surface text-gray-500",
    dot: "bg-gray-500",
  },
};

export function StatusBadge({ status }: { readonly status: ArticleStatus }) {
  const tone = TONE[status];
  return (
    <span
      className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 font-mono text-xs ${tone.classes}`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${tone.dot}`} />
      {tone.label}
    </span>
  );
}
