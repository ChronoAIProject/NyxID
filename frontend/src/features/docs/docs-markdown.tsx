import { useMemo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkDirective from "remark-directive";
import rehypeSlug from "rehype-slug";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import { Callout } from "./components/callout";

// Allow class names (callout markers + `language-*` code classes) through the
// sanitizer. Content is our own authored docs, but we keep sanitize for hygiene.
const schema = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    "*": [...(defaultSchema.attributes?.["*"] ?? []), "className"],
  },
};

// Turn `:::note` / `:::tip` / `:::warning` / `:::info` / `:::danger` container
// directives into `<div class="callout callout-<kind>">`, which the components
// map below renders as a <Callout>. Implemented without unist-util-visit to
// avoid an extra dependency.
const CALLOUT_KINDS = new Set(["note", "info", "tip", "warning", "danger"]);
function remarkCallouts() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const walk = (node: any) => {
    if (!node || typeof node !== "object") return;
    if (
      (node.type === "containerDirective" || node.type === "leafDirective") &&
      CALLOUT_KINDS.has(node.name)
    ) {
      node.data = node.data ?? {};
      node.data.hName = "div";
      node.data.hProperties = { className: `callout callout-${node.name}` };
    }
    if (Array.isArray(node.children)) node.children.forEach(walk);
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return (tree: any) => walk(tree);
}

const REMARK_PLUGINS = [remarkGfm, remarkDirective, remarkCallouts];
// sanitize before slug so the generated heading ids survive unclobbered.
const REHYPE_PLUGINS = [[rehypeSanitize, schema], rehypeSlug];

/**
 * Renders docs markdown with the app's design language (Mona Sans headings,
 * semantic surface/text tokens), callouts, and heading ids (for the on-page
 * TOC). Relative image srcs resolve against `baseHref` (the public dir of the
 * current doc, e.g. `/docs/web/getting-started/`).
 */
export function DocMarkdown({
  markdown,
  baseHref,
}: {
  readonly markdown: string;
  readonly baseHref: string;
}) {
  const components = useMemo<Components>(
    () => ({
      h1: ({ children, id }) => (
        <h1 id={id} className="mt-10 mb-4 scroll-mt-24 font-display text-3xl font-bold tracking-tight text-foreground md:text-4xl">{children}</h1>
      ),
      h2: ({ children, id }) => (
        <h2 id={id} className="mt-10 mb-3 scroll-mt-24 font-display text-2xl font-semibold tracking-tight text-foreground">{children}</h2>
      ),
      h3: ({ children, id }) => (
        <h3 id={id} className="mt-7 mb-2 scroll-mt-24 font-mono text-base font-medium text-foreground">{children}</h3>
      ),
      p: ({ children }) => <p className="mb-4 leading-relaxed text-muted-foreground">{children}</p>,
      a: ({ href, children }) => {
        const external = typeof href === "string" && href.startsWith("http");
        return (
          <a
            href={href}
            target={external ? "_blank" : undefined}
            rel={external ? "noopener noreferrer" : undefined}
            className="text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 underline-offset-4 transition-colors hover:text-nyx-secondary-300 hover:decoration-nyx-secondary-300"
          >
            {children}
          </a>
        );
      },
      ul: ({ children }) => (
        <ul className="mb-4 list-disc space-y-1.5 pl-6 leading-relaxed text-muted-foreground marker:text-nyx-secondary-400">{children}</ul>
      ),
      ol: ({ children }) => (
        <ol className="mb-4 list-decimal space-y-1.5 pl-6 leading-relaxed text-muted-foreground marker:font-mono marker:text-text-tertiary">{children}</ol>
      ),
      li: ({ children }) => <li className="pl-1">{children}</li>,
      strong: ({ children }) => <strong className="font-semibold text-foreground">{children}</strong>,
      em: ({ children }) => <em className="text-foreground italic">{children}</em>,
      hr: () => <hr className="border-border my-8" />,
      blockquote: ({ children }) => (
        <blockquote className="border-nyx-secondary-400/50 my-5 border-l-2 pl-4 text-muted-foreground italic">{children}</blockquote>
      ),
      table: ({ children }) => (
        <div className="border-border bg-card my-5 overflow-x-auto rounded-xl border">
          <table className="w-full text-left text-sm text-muted-foreground">{children}</table>
        </div>
      ),
      thead: ({ children }) => (
        <thead className="border-border border-b font-mono text-xs text-text-tertiary uppercase">{children}</thead>
      ),
      tr: ({ children }) => <tr className="border-border border-b last:border-0">{children}</tr>,
      th: ({ children }) => <th className="px-4 py-2.5 font-medium text-foreground">{children}</th>,
      td: ({ children }) => <td className="px-4 py-2.5">{children}</td>,
      img: ({ src, alt }) => {
        let resolved = typeof src === "string" ? src : "";
        if (resolved && !resolved.startsWith("http") && !resolved.startsWith("/")) {
          resolved = baseHref + resolved;
        }
        return (
          <img
            src={resolved}
            alt={alt ?? ""}
            className="border-border my-5 w-full rounded-xl border"
            loading="lazy"
          />
        );
      },
      code: ({ className, children }) => {
        const isBlock = typeof className === "string" && className.startsWith("language-");
        if (isBlock) return <code className={className}>{children}</code>;
        return (
          <code className="bg-muted text-foreground rounded px-1.5 py-0.5 font-mono text-[0.85em]">{children}</code>
        );
      },
      pre: ({ children }) => (
        <pre className="border-border bg-muted my-5 overflow-x-auto rounded-xl border p-4 font-mono text-sm leading-relaxed text-foreground">
          {children}
        </pre>
      ),
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      div: ({ className, children }: any) => {
        if (typeof className === "string" && className.includes("callout")) {
          const type = /callout-(\w+)/.exec(className)?.[1] ?? "note";
          return <Callout type={type}>{children}</Callout>;
        }
        return <div className={className}>{children}</div>;
      },
    }),
    [baseHref],
  );

  return (
    <div className="text-muted-foreground">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        rehypePlugins={REHYPE_PLUGINS as any}
        components={components}
      >
        {markdown}
      </ReactMarkdown>
    </div>
  );
}
