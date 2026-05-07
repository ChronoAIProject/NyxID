// Mock API for the blog feature.
//
// In production these will be HTTP GETs against a CDN-fronted endpoint
// returning Directus's `{ data: ... }` response shape (see types.ts). Until
// that's wired, these helpers stand in: same async signature, simulated
// latency, same response shape.
//
// SHIP NOTE: this file (and `./mock-data`) are bundled into the public client.
// Before going to production, replace these helpers with real `fetch()` calls
// against the CDN, and remove `./mock-data.ts` entirely. Otherwise unpublished
// (`draft` / `in_review`) articles are recoverable from the JS bundle, which
// defeats the whole point of `/preview/<uuid>` being a server-enforced secret.

import { MOCK_ARTICLES } from "./mock-data";
import type {
  BlogArticle,
  DirectusItemResponse,
  DirectusListResponse,
} from "./types";

const SIMULATED_LATENCY_MS = 250;

function delay<T>(value: T): Promise<T> {
  return new Promise((resolve) => {
    setTimeout(() => resolve(value), SIMULATED_LATENCY_MS);
  });
}

function byPublishedDesc(a: BlogArticle, b: BlogArticle): number {
  const aTs = a.published_at ?? "";
  const bTs = b.published_at ?? "";
  return bTs.localeCompare(aTs);
}

export async function fetchPublishedArticles(): Promise<
  DirectusListResponse<BlogArticle>
> {
  const data = MOCK_ARTICLES.filter((a) => a.status === "published").sort(
    byPublishedDesc,
  );
  return delay({ data });
}

export async function fetchArticleBySlug(
  slug: string,
): Promise<DirectusItemResponse<BlogArticle>> {
  const data =
    MOCK_ARTICLES.find((a) => a.slug === slug && a.status === "published") ??
    null;
  return delay({ data });
}

// Preview reads by UUID and returns regardless of status — that's the whole
// point of the preview URL secret.
export async function fetchArticleById(
  id: string,
): Promise<DirectusItemResponse<BlogArticle>> {
  const data = MOCK_ARTICLES.find((a) => a.id === id) ?? null;
  return delay({ data });
}
