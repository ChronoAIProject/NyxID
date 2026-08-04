import { Link } from "@tanstack/react-router";

import type { BlogArticle } from "../types";
import { estimateReadingMinutes } from "../utils";
import { ArticleBody } from "./article-body";
import { ArticleMeta } from "./article-meta";
import { StatusBadge } from "./status-badge";

interface ArticleViewProps {
  readonly article: BlogArticle;
  readonly previewBanner?: boolean;
}

export function ArticleView({ article, previewBanner = false }: ArticleViewProps) {
  const readingMinutes = estimateReadingMinutes(article.body);

  return (
    <article className="px-6 pt-12 pb-24">
      <div className="mx-auto max-w-3xl">
        {previewBanner ? <PreviewBanner article={article} /> : null}

        <Link
          to="/blog"
          className="font-mono text-xs tracking-wider text-gray-500 transition-colors hover:text-primary"
        >
          ← Back to Field Notes
        </Link>

        {article.tags[0] ? (
          <p className="mt-8 font-mono text-[11px] tracking-[1.5px] text-primary uppercase">
            {article.tags[0].name}
            {article.series ? (
              <>
                {" · "}
                <span className="text-gray-500">{article.series.name}</span>
              </>
            ) : null}
          </p>
        ) : null}

        <h1 className="mt-4 font-serif text-3xl leading-tight text-white md:text-5xl">
          {article.title}
        </h1>

        <p className="mt-6 text-lg leading-relaxed text-gray-300">
          {article.description}
        </p>

        <div className="mt-8">
          <ArticleMeta
            author={article.author}
            publishedAt={article.published_at}
            readingMinutes={readingMinutes}
          />
        </div>

        {article.hero_image ? (
          <div className="border-landing-border-subtle bg-landing-surface mt-10 aspect-[16/9] overflow-hidden rounded-2xl border">
            <img
              src={article.hero_image.url}
              alt={article.hero_image.alt ?? article.title}
              className="h-full w-full object-cover"
            />
          </div>
        ) : null}

        <div className="mt-10">
          <ArticleBody markdown={article.body} />
        </div>

        {article.tags.length > 0 ? (
          <div className="border-landing-border-subtle mt-12 flex flex-wrap gap-2 border-t pt-8">
            {article.tags.map((tag) => (
              <span
                key={tag.id}
                className="border-landing-border-subtle bg-landing-surface rounded-full border px-3 py-1 font-mono text-xs text-gray-400"
              >
                #{tag.slug}
              </span>
            ))}
          </div>
        ) : null}

        <AuthorBio article={article} />
      </div>
    </article>
  );
}

function PreviewBanner({ article }: { readonly article: BlogArticle }) {
  return (
    <div className="bg-primary/5 mb-10 flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 px-4 py-3">
      <StatusBadge status={article.status} />
      <span className="text-sm text-gray-300">
        Preview mode — visible only via the secret preview URL.
      </span>
    </div>
  );
}

function AuthorBio({ article }: { readonly article: BlogArticle }) {
  const author = article.author;
  if (!author.description) return null;

  return (
    <div className="border-landing-border-subtle bg-landing-surface mt-10 flex flex-col gap-4 rounded-2xl border p-6 sm:flex-row sm:items-start sm:gap-5 sm:p-8">
      <span className="from-nyx-200 to-nyx-700 grid h-14 w-14 shrink-0 place-items-center rounded-full bg-gradient-to-br text-base font-semibold text-white">
        {(author.first_name?.[0] ?? "").toUpperCase()}
        {(author.last_name?.[0] ?? "").toUpperCase()}
      </span>
      <div>
        <p className="font-mono text-[11px] tracking-[1.5px] text-primary uppercase">
          Written by
        </p>
        <p className="mt-1 font-serif text-xl text-white">
          {author.first_name} {author.last_name}
        </p>
        <p className="mt-2 text-sm leading-relaxed text-gray-300">
          {author.description}
        </p>
      </div>
    </div>
  );
}
