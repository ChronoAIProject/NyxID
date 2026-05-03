import type { DirectusUser } from "../types";

interface ArticleMetaProps {
  readonly author: DirectusUser;
  readonly publishedAt: string | null;
  readonly readingMinutes: number;
}

function formatDate(iso: string | null): string {
  if (!iso) return "Unpublished";
  const date = new Date(iso);
  return date.toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function authorInitials(author: DirectusUser): string {
  const f = author.first_name?.[0] ?? "";
  const l = author.last_name?.[0] ?? "";
  return `${f}${l}`.toUpperCase() || "?";
}

export function ArticleMeta({
  author,
  publishedAt,
  readingMinutes,
}: ArticleMetaProps) {
  return (
    <div className="border-landing-border-subtle flex flex-wrap items-center gap-x-6 gap-y-3 border-y py-5">
      <div className="flex items-center gap-3">
        <span className="from-void-300 to-void-700 grid h-9 w-9 place-items-center rounded-full bg-gradient-to-br text-xs font-semibold text-white">
          {authorInitials(author)}
        </span>
        <div className="leading-tight">
          <div className="text-sm font-medium text-white">
            {author.first_name} {author.last_name}
          </div>
          {author.title ? (
            <div className="font-mono text-xs text-gray-500">
              {author.title}
            </div>
          ) : null}
        </div>
      </div>
      <span className="font-mono text-xs text-gray-500">
        {formatDate(publishedAt)}
      </span>
      <span className="font-mono text-xs text-gray-500">
        {readingMinutes} min read
      </span>
    </div>
  );
}
