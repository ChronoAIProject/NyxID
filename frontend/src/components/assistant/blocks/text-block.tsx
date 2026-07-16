import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";

const ALLOWED_ELEMENTS = ["p", "strong", "em", "code", "a", "br"];

function allowedHref(href: string | undefined): string | null {
  if (!href) return null;
  return href.startsWith("https:") || href.startsWith("mailto:") ? href : null;
}

const COMPONENTS: Components = {
  p: ({ children }) => (
    <p className="whitespace-pre-wrap leading-[1.65] text-foreground/90">
      {children}
    </p>
  ),
  strong: ({ children }) => (
    <strong className="font-semibold text-foreground">{children}</strong>
  ),
  em: ({ children }) => <em className="text-foreground italic">{children}</em>,
  code: ({ children }) => (
    <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground">
      {children}
    </code>
  ),
  a: ({ href, children }) => {
    const safeHref = allowedHref(href);
    if (!safeHref) {
      return <span className="text-muted-foreground">{children}</span>;
    }
    return (
      <a
        href={safeHref}
        target={safeHref.startsWith("https:") ? "_blank" : undefined}
        rel="noopener noreferrer"
        className="text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 underline-offset-2 hover:decoration-nyx-secondary-400"
      >
        {children}
      </a>
    );
  },
};

export function TextBlock({ text }: { readonly text: string }) {
  const escapedHtml = text.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  return (
    <div className="text-[13px]">
      <ReactMarkdown
        allowedElements={ALLOWED_ELEMENTS}
        rehypePlugins={[rehypeSanitize]}
        urlTransform={(url) => allowedHref(url) ?? ""}
        components={COMPONENTS}
      >
        {escapedHtml}
      </ReactMarkdown>
    </div>
  );
}
