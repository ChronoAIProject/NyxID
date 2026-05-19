import { Link } from "@tanstack/react-router";

import type { BlogArticle } from "../types";
import { estimateReadingMinutes } from "../utils";

function formatShortDate(iso: string | null): string {
  if (!iso) return "Draft";
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
}

export function ArticleCard({ article }: { readonly article: BlogArticle }) {
  const readingMinutes = estimateReadingMinutes(article.body);
  const primaryTag = article.tags[0]?.name;

  return (
    <Link
      to="/blog/$slug"
      params={{ slug: article.slug }}
      className="group border-landing-border-subtle bg-landing-surface flex flex-col overflow-hidden rounded-2xl border transition-colors hover:border-white/[0.15]"
    >
      {article.hero_image ? (
        <div className="bg-landing-surface-raised aspect-[16/9] overflow-hidden">
          <img
            src={article.hero_image.url}
            alt={article.hero_image.alt ?? ""}
            loading="lazy"
            className="h-full w-full object-cover opacity-80 transition-opacity duration-500 group-hover:opacity-100"
          />
        </div>
      ) : null}

      <div className="flex flex-1 flex-col gap-3 p-6">
        {primaryTag ? (
          <span className="font-mono text-[11px] tracking-[1.5px] text-primary uppercase">
            {primaryTag}
          </span>
        ) : null}
        <h3 className="font-mono text-lg leading-snug font-medium text-white">
          {article.title}
        </h3>
        <p className="text-sm leading-relaxed text-gray-300">
          {article.description}
        </p>
        <div className="border-landing-border-subtle mt-auto flex items-center justify-between border-t pt-4 font-mono text-xs text-gray-500">
          <span>
            {article.author.first_name} {article.author.last_name}
          </span>
          <span>
            {formatShortDate(article.published_at)} · {readingMinutes} min
          </span>
        </div>
      </div>
    </Link>
  );
}
