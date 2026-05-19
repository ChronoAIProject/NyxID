import { useEffect, useState } from "react";
import { useParams } from "@tanstack/react-router";

import { ArticleNotFound } from "./components/article-not-found";
import { ArticleView } from "./components/article-view";
import { BlogShell } from "./components/blog-shell";
import { fetchArticleBySlug } from "./mock-api";
import type { BlogArticle } from "./types";

interface FetchResult {
  readonly slug: string;
  readonly article: BlogArticle | null;
  readonly error: string | null;
}

export function BlogDetailPage() {
  const { slug } = useParams({ from: "/blog/$slug" });
  const [result, setResult] = useState<FetchResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchArticleBySlug(slug)
      .then((res) => {
        if (cancelled) return;
        setResult({ slug, article: res.data, error: null });
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setResult({
          slug,
          article: null,
          error: e instanceof Error ? e.message : "Failed to load article.",
        });
      });
    return () => {
      cancelled = true;
    };
  }, [slug]);

  // The currently displayed result is stale (or absent) until the most recent
  // fetch settles for the param the URL is now showing.
  const isLoading = result === null || result.slug !== slug;

  return (
    <BlogShell>
      {isLoading ? <ArticleSkeleton /> : null}
      {!isLoading && result.error ? (
        <ErrorState message={result.error} />
      ) : null}
      {!isLoading && !result.error && result.article === null ? (
        <ArticleNotFound />
      ) : null}
      {!isLoading && !result.error && result.article ? (
        <ArticleView article={result.article} />
      ) : null}
    </BlogShell>
  );
}

function ArticleSkeleton() {
  return (
    <div className="px-6 pt-12 pb-24" aria-busy="true">
      <div className="mx-auto max-w-3xl space-y-6">
        <div className="bg-landing-surface-raised h-3 w-32 animate-pulse rounded" />
        <div className="bg-landing-surface-raised h-12 w-full animate-pulse rounded" />
        <div className="bg-landing-surface-raised h-5 w-4/5 animate-pulse rounded" />
        <div className="bg-landing-surface-raised aspect-[16/9] animate-pulse rounded-2xl" />
        <div className="space-y-3">
          <div className="bg-landing-surface-raised h-4 w-full animate-pulse rounded" />
          <div className="bg-landing-surface-raised h-4 w-11/12 animate-pulse rounded" />
          <div className="bg-landing-surface-raised h-4 w-10/12 animate-pulse rounded" />
        </div>
      </div>
    </div>
  );
}

function ErrorState({ message }: { readonly message: string }) {
  return (
    <div className="px-6 pt-20 pb-24">
      <div
        role="alert"
        className="bg-destructive/5 mx-auto max-w-md rounded-2xl border border-destructive/40 p-10 text-center"
      >
        <p className="font-mono text-xs tracking-widest text-destructive uppercase">
          Failed to load
        </p>
        <p className="mt-3 text-sm text-gray-300">{message}</p>
      </div>
    </div>
  );
}
