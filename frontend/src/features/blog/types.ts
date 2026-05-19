// Mirrors the Directus blog_articles schema documented in DESIGN.
// M2O / M2M relations are modelled as embedded objects, matching how a
// Directus read with `?fields=*.*` returns expanded relations.

export type ArticleStatus = "draft" | "in_review" | "published" | "archived";

export interface DirectusFile {
  readonly id: string;
  readonly filename_disk: string;
  readonly url: string;
  readonly width?: number;
  readonly height?: number;
  readonly alt?: string;
}

export interface DirectusUser {
  readonly id: string;
  readonly first_name: string;
  readonly last_name: string;
  readonly email: string;
  readonly title?: string;
  readonly description?: string;
  readonly avatar?: DirectusFile | null;
}

export interface Tag {
  readonly id: string;
  readonly slug: string;
  readonly name: string;
}

export interface Series {
  readonly id: string;
  readonly slug: string;
  readonly name: string;
  readonly description?: string;
}

export interface Product {
  readonly id: string;
  readonly name: string;
  readonly site_url: string;
  readonly site_github_repo: string;
  readonly site_dispatch_event_type: string;
  readonly content_path: string;
}

export interface BlogArticle {
  readonly id: string;
  readonly product: Product;
  readonly slug: string;
  readonly title: string;
  readonly description: string;
  readonly body: string;
  readonly tags: readonly Tag[];
  readonly series: Series | null;
  readonly author: DirectusUser;
  readonly hero_image: DirectusFile | null;
  readonly published_at: string | null;
  readonly status: ArticleStatus;
  readonly content_commit_sha: string;
  readonly content_url: string;
}

export interface DirectusListResponse<T> {
  readonly data: readonly T[];
}

export interface DirectusItemResponse<T> {
  readonly data: T | null;
}
