import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";

// Everything the assistant may stream should actually render (issue: results
// were flattened). rehype-sanitize still scrubs attributes/urls; these are the
// safe block/inline tags we style.
const ALLOWED_ELEMENTS = [
  "p",
  "strong",
  "em",
  "del",
  "code",
  "pre",
  "a",
  "br",
  "ul",
  "ol",
  "li",
  "h1",
  "h2",
  "h3",
  "h4",
  "blockquote",
  "hr",
];

// Inter-block rhythm without a container `space-y-*` — that would also push the
// trailing streaming caret onto its own line. First block hugs the top.
const BLOCK = "mt-2 first:mt-0";

function allowedHref(href: string | undefined): string | null {
  if (!href) return null;
  return href.startsWith("https:") || href.startsWith("mailto:") ? href : null;
}

const COMPONENTS: Components = {
  p: ({ children }) => (
    <p className={`${BLOCK} whitespace-pre-wrap leading-[1.65] text-foreground/90`}>
      {children}
    </p>
  ),
  strong: ({ children }) => (
    <strong className="font-semibold text-foreground">{children}</strong>
  ),
  em: ({ children }) => <em className="text-foreground italic">{children}</em>,
  del: ({ children }) => (
    <del className="text-muted-foreground line-through">{children}</del>
  ),
  h1: ({ children }) => (
    <h1 className={`${BLOCK} text-[15px] font-semibold text-foreground`}>
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className={`${BLOCK} text-[14px] font-semibold text-foreground`}>
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className={`${BLOCK} text-[13px] font-semibold text-foreground`}>
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4 className={`${BLOCK} text-[13px] font-medium text-foreground`}>
      {children}
    </h4>
  ),
  ul: ({ children }) => (
    <ul
      className={`${BLOCK} ml-4 list-disc space-y-1 text-foreground/90 marker:text-text-tertiary`}
    >
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol
      className={`${BLOCK} ml-4 list-decimal space-y-1 text-foreground/90 marker:text-text-tertiary`}
    >
      {children}
    </ol>
  ),
  li: ({ children }) => (
    <li className="leading-[1.6] [&>p]:mt-0 [&>p]:inline">{children}</li>
  ),
  blockquote: ({ children }) => (
    <blockquote
      className={`${BLOCK} border-l-2 border-hairline pl-3 text-muted-foreground`}
    >
      {children}
    </blockquote>
  ),
  hr: () => <hr className={`${BLOCK} border-hairline`} />,
  pre: ({ children }) => (
    <pre
      className={`${BLOCK} overflow-x-auto rounded-lg border border-hairline bg-overlay px-3 py-2 font-mono text-[11px] leading-relaxed text-foreground`}
    >
      {children}
    </pre>
  ),
  code: ({ className, children }) => {
    // Fenced blocks carry `language-*`; the surrounding <pre> owns the styling.
    if (typeof className === "string" && className.startsWith("language-")) {
      return <code className={className}>{children}</code>;
    }
    return (
      <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground">
        {children}
      </code>
    );
  },
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

export function TextBlock({
  text,
  streaming = false,
}: {
  readonly text: string;
  /**
   * True for the actively-streaming text block: renders a blinking caret glued
   * to the end of the last line so the turn reads as "still writing" rather
   * than freezing between chunks.
   */
  readonly streaming?: boolean;
}) {
  const escapedHtml = text.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  return (
    <div
      className={`text-[13px] ${streaming ? "[&>p:last-of-type]:inline" : ""}`}
    >
      <ReactMarkdown
        allowedElements={ALLOWED_ELEMENTS}
        rehypePlugins={[rehypeSanitize]}
        urlTransform={(url) => allowedHref(url) ?? ""}
        components={COMPONENTS}
      >
        {escapedHtml}
      </ReactMarkdown>
      {streaming ? (
        <span
          aria-hidden
          className="ml-0.5 inline-block h-[0.95em] w-[2px] translate-y-[2px] animate-pulse rounded-full bg-nyx-secondary-400 align-baseline"
        />
      ) : null}
    </div>
  );
}
