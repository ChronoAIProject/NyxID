import { DocMarkdown } from "../docs/docs-markdown";
import { DocsLayout } from "../docs/docs-layout-view";
import { DocsIndexPage } from "../docs/docs-index-page";
import { docTabForSlug } from "../docs/manifest";
export function parseDoc(markdown: string) {
  const front = markdown.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n*/);
  return {
    title: front?.[1]?.match(/^title:\s*"?(.+?)"?\s*$/m)?.[1] ?? "NyxID Docs",
    description:
      front?.[1]?.match(/^description:\s*"?(.+?)"?\s*$/m)?.[1] ??
      "NyxID documentation",
    body: markdown.slice(front?.[0].length ?? 0).replace(/^#\s+.+\n+/, ""),
  };
}
export function PublicDocument({
  slug,
  markdown,
}: {
  slug: string;
  markdown?: string;
}) {
  if (!slug) return <DocsIndexPage />;
  if (markdown === undefined)
    throw new Error("Missing approved documentation body");
  const doc = parseDoc(markdown),
    tab = docTabForSlug(slug);
  return (
    <DocsLayout activeTab={tab === "shared" ? "ai" : tab} currentSlug={slug}>
      <article id="website-main" className="max-w-3xl">
        <h1 className="mb-6 font-display text-3xl font-bold">{doc.title}</h1>
        <DocMarkdown
          markdown={doc.body}
          baseHref={`/docs/${slug.split("/").slice(0, -1).join("/")}/`}
        />
      </article>
    </DocsLayout>
  );
}
