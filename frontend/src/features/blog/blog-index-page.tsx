import { useEffect, useState } from "react";

import { ArticleCard } from "./components/article-card";
import { BlogShell } from "./components/blog-shell";
import { fetchPublishedArticles } from "./mock-api";
import type { BlogArticle } from "./types";

export function BlogIndexPage() {
  const [articles, setArticles] = useState<readonly BlogArticle[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchPublishedArticles()
      .then((res) => {
        if (!cancelled) setArticles(res.data);
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : "Failed to load articles.");
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <BlogShell>
      <section className="px-6 pt-20 pb-12">
        <div className="mx-auto max-w-6xl">
          <p className="font-mono text-xs tracking-widest text-primary uppercase">
            The NyxID Journal
          </p>
          <h1 className="mt-4 max-w-3xl font-serif text-4xl leading-tight text-white md:text-6xl">
            Field notes from the threshold
          </h1>
          <p className="mt-6 max-w-2xl text-lg leading-relaxed text-gray-300">
            Dispatches from the team building the lock between you and
            everything that asks. Engineering deep-dives, security write-ups,
            design decisions — and the occasional confession.
          </p>
        </div>
      </section>

      <section className="px-6 pb-24">
        <div className="mx-auto max-w-6xl">
          {error ? <ErrorState message={error} /> : null}
          {!error && articles === null ? <LoadingGrid /> : null}
          {!error && articles !== null && articles.length === 0 ? (
            <EmptyState />
          ) : null}
          {!error && articles !== null && articles.length > 0 ? (
            <ArticleGrid articles={articles} />
          ) : null}
        </div>
      </section>
    </BlogShell>
  );
}

function ArticleGrid({
  articles,
}: {
  readonly articles: readonly BlogArticle[];
}) {
  return (
    <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
      {articles.map((article) => (
        <ArticleCard key={article.id} article={article} />
      ))}
    </div>
  );
}

function LoadingGrid() {
  return (
    <div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3" aria-busy="true">
      {[0, 1, 2, 3, 4, 5].map((i) => (
        <div
          key={i}
          className="border-landing-border-subtle bg-landing-surface flex flex-col gap-4 overflow-hidden rounded-2xl border"
        >
          <div className="bg-landing-surface-raised aspect-[16/9] animate-pulse" />
          <div className="flex flex-col gap-3 p-6">
            <div className="bg-landing-surface-raised h-3 w-20 animate-pulse rounded" />
            <div className="bg-landing-surface-raised h-5 w-4/5 animate-pulse rounded" />
            <div className="bg-landing-surface-raised h-4 w-full animate-pulse rounded" />
            <div className="bg-landing-surface-raised h-4 w-2/3 animate-pulse rounded" />
          </div>
        </div>
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="border-landing-border-subtle bg-landing-surface mx-auto max-w-md rounded-2xl border p-10 text-center">
      <p className="font-serif text-2xl text-white">Nothing to show yet.</p>
      <p className="mt-3 text-sm text-gray-400">
        Field notes will appear here as they're published.
      </p>
    </div>
  );
}

function ErrorState({ message }: { readonly message: string }) {
  return (
    <div
      role="alert"
      className="bg-destructive/5 mx-auto max-w-md rounded-2xl border border-destructive/40 p-10 text-center"
    >
      <p className="font-mono text-xs tracking-widest text-destructive uppercase">
        Something went wrong
      </p>
      <p className="mt-3 text-sm text-gray-300">{message}</p>
    </div>
  );
}
