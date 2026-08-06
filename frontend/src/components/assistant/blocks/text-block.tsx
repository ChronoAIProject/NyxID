import {
  createContext,
  memo,
  useContext,
  useMemo,
  type ComponentProps,
  type CSSProperties,
} from "react";
import type { Root } from "mdast";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { useSmoothReveal } from "@/hooks/use-smooth-reveal";
import { splitStableMarkdown } from "@/lib/assistant/markdown-stream";

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
  "h5",
  "h6",
  "blockquote",
  "hr",
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
  "input",
  "img",
  "span",
  "sup",
  "section",
];

// Keep the sanitizer explicit at the two GFM extension points used here.
// The required rule also forces task-list inputs disabled even if a future
// renderer accidentally omits the prop.
function protocolCaseVariants(protocol: string): string[] {
  let variants = [""];
  for (const character of protocol) {
    variants = variants.flatMap((prefix) => [
      `${prefix}${character.toLowerCase()}`,
      `${prefix}${character.toUpperCase()}`,
    ]);
  }
  return variants;
}

const SANITIZE_SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    input: [
      ...(defaultSchema.attributes?.input ?? []),
      "checked",
    ],
    th: [...(defaultSchema.attributes?.th ?? []), "align"],
    td: [...(defaultSchema.attributes?.td ?? []), "align"],
  },
  protocols: {
    ...defaultSchema.protocols,
    href: [
      ...protocolCaseVariants("https"),
      ...protocolCaseVariants("mailto"),
    ],
  },
};

// Inter-block rhythm without a container `space-y-*` — that would also push the
// trailing streaming caret onto its own line. First block hugs the top.
const BLOCK = "mt-2 first:mt-0";

function allowedHref(href: string | undefined): string | null {
  if (!href) return null;
  const normalized = href.trim().toLowerCase();
  return normalized.startsWith("https:") ||
    normalized.startsWith("mailto:") ||
    normalized.startsWith("#")
    ? href
    : null;
}

function isHttpsHref(href: string): boolean {
  return href.trim().toLowerCase().startsWith("https:");
}

/** Literalize raw HTML and flatten images nested inside Markdown links. */
function remarkSafeModelContent() {
  return (tree: Root) => {
    function visit(node: unknown, insideLink = false): void {
      if (!node || typeof node !== "object") return;
      const candidate = node as {
        type?: string;
        value?: string;
        url?: string;
        alt?: string;
        children?: unknown[];
        data?: unknown;
      };
      if (candidate.type === "html") {
        candidate.type = "text";
        return;
      }
      if (candidate.type === "image" && insideLink) {
        Object.assign(candidate, {
          type: "emphasis",
          children: [{ type: "text", value: candidate.alt?.trim() || "Image" }],
          data: {
            hName: "span",
            hProperties: { title: candidate.url },
          },
        });
        return;
      }
      const childrenInsideLink = insideLink || candidate.type === "link";
      candidate.children?.forEach((child) => visit(child, childrenInsideLink));
    }

    visit(tree);
  };
}

function tableCellStyle(
  style: CSSProperties | undefined,
  align: string | undefined,
): CSSProperties | undefined {
  if (style) return style;
  if (
    align === "left" ||
    align === "center" ||
    align === "right" ||
    align === "justify"
  ) {
    return { textAlign: align };
  }
  return undefined;
}

const InsideFootnoteSectionContext = createContext(false);

function MarkdownHeadingTwo({ children }: ComponentProps<"h2">) {
  const insideFootnotes = useContext(InsideFootnoteSectionContext);
  return insideFootnotes ? (
    <h2 className="sr-only">{children}</h2>
  ) : (
    <h2 className={`${BLOCK} text-[14px] font-semibold text-foreground`}>
      {children}
    </h2>
  );
}

const COMPONENTS: Components = {
  p: ({ children }) => (
    <p
      className={`${BLOCK} whitespace-pre-wrap leading-[1.65] text-foreground/90`}
    >
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
  h2: MarkdownHeadingTwo,
  h3: ({ children }) => (
    <h3 className={`${BLOCK} text-[13.5px] font-semibold text-foreground`}>
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4 className={`${BLOCK} text-[13px] font-medium text-foreground`}>
      {children}
    </h4>
  ),
  h5: ({ children }) => (
    <h5 className={`${BLOCK} text-[12.5px] font-semibold text-foreground`}>
      {children}
    </h5>
  ),
  h6: ({ children }) => (
    <h6 className={`${BLOCK} text-[12px] font-medium text-foreground`}>
      {children}
    </h6>
  ),
  ul: ({ children, className }) => (
    <ul
      className={`${BLOCK} space-y-1 text-foreground/90 marker:text-text-tertiary ${
        className?.includes("contains-task-list")
          ? "ml-0 list-none"
          : "ml-4 list-disc"
      }`}
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
  table: ({ children }) => (
    <div
      className={`${BLOCK} overflow-x-auto rounded-lg border border-hairline`}
    >
      <table className="w-full border-collapse text-left text-[12px] text-foreground/90">
        {children}
      </table>
    </div>
  ),
  thead: ({ children }) => (
    <thead className="border-b border-hairline bg-overlay text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary">
      {children}
    </thead>
  ),
  tbody: ({ children }) => (
    <tbody className="divide-y divide-hairline">{children}</tbody>
  ),
  tr: ({ children }) => <tr>{children}</tr>,
  th: ({ children, style, align }) => (
    <th
      className="px-3 py-2 text-left font-medium text-foreground"
      style={tableCellStyle(style, align)}
    >
      {children}
    </th>
  ),
  td: ({ children, style, align }) => (
    <td
      className="px-3 py-2 align-top text-[12px]"
      style={tableCellStyle(style, align)}
    >
      {children}
    </td>
  ),
  input: ({ type, checked }) => (
    <input
      type={type === "checkbox" ? "checkbox" : undefined}
      checked={Boolean(checked)}
      disabled
      readOnly
      className="mr-1.5 h-3.5 w-3.5 translate-y-[2px] accent-nyx-secondary-400"
    />
  ),
  sup: ({ children }) => (
    <sup className="text-[10px] leading-none text-nyx-secondary-400">
      {children}
    </sup>
  ),
  section: ({ children }) => (
    <section
      className={`${BLOCK} border-t border-hairline pt-2 text-[11px] text-foreground/90`}
    >
      <div className="mb-1 text-[10px] font-medium text-muted-foreground">
        Footnotes
      </div>
      <InsideFootnoteSectionContext.Provider value>
        {children}
      </InsideFootnoteSectionContext.Provider>
    </section>
  ),
  a: ({ href, children }) => {
    const safeHref = allowedHref(href);
    if (!safeHref) {
      return <span className="text-muted-foreground">{children}</span>;
    }
    return (
      <a
        href={safeHref}
        target={isHttpsHref(safeHref) ? "_blank" : undefined}
        rel="noopener noreferrer"
        className="text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 underline-offset-2 hover:decoration-nyx-secondary-400"
      >
        {children}
      </a>
    );
  },
  // Remote model-provided images stay links to prevent tracking/exfiltration.
  img: ({ src, alt }) => {
    const safeHref = allowedHref(typeof src === "string" ? src : undefined);
    const label = alt?.trim() || "Image";
    if (!safeHref) {
      return <span className="text-muted-foreground">{label}</span>;
    }
    return (
      <a
        href={safeHref}
        target={isHttpsHref(safeHref) ? "_blank" : undefined}
        rel="noopener noreferrer"
        className="text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 underline-offset-2 hover:decoration-nyx-secondary-400"
      >
        {label}
      </a>
    );
  },
};

// Hoisted so the plugin arrays keep a stable identity across renders — rebuilt
// inline they defeat every memo react-markdown applies internally.
type MarkdownProps = ComponentProps<typeof ReactMarkdown>;
const REMARK_PLUGINS: MarkdownProps["remarkPlugins"] = [
  remarkGfm,
  remarkSafeModelContent,
];
const REHYPE_PLUGINS: MarkdownProps["rehypePlugins"] = [
  [rehypeSanitize, SANITIZE_SCHEMA],
];
const urlTransform: MarkdownProps["urlTransform"] = (url) =>
  allowedHref(url) ?? "";

/**
 * One parse, memoized on the text it parsed.
 *
 * react-markdown holds no internal cache: a parent re-render re-runs remark,
 * rehype-sanitize and the whole JSX build for text that has not changed by a
 * character. Measured at ~15 ms for a 4 000-character answer — per re-render,
 * per block. Memoizing here is what keeps a settled transcript out of the
 * streaming block's frame budget.
 *
 * Rendered WITHOUT a wrapper element, deliberately: react-markdown emits its
 * blocks into a fragment, so a split block's prefix and tail remain siblings
 * under one parent and the `first:mt-0` rhythm still resolves across the seam.
 */
const MarkdownSegment = memo(function MarkdownSegment({
  text,
}: {
  readonly text: string;
}) {
  return (
    <ReactMarkdown
      allowedElements={ALLOWED_ELEMENTS}
      remarkPlugins={REMARK_PLUGINS}
      rehypePlugins={REHYPE_PLUGINS}
      urlTransform={urlTransform}
      components={COMPONENTS}
    >
      {text}
    </ReactMarkdown>
  );
});

export const TextBlock = memo(function TextBlock({
  text,
  streaming = false,
}: {
  readonly text: string;
  /**
   * True for the actively-streaming text block: paces the reveal against the
   * frame clock and renders a blinking caret glued to the end of the last line,
   * so the turn reads as "still writing" rather than freezing between chunks.
   */
  readonly streaming?: boolean;
}) {
  const revealed = useSmoothReveal(text, streaming);
  // Everything above the last settled block is frozen and memoized; only the
  // live tail is re-parsed per frame. Splitting is refused wherever it could
  // change meaning, in which case this is a whole-text parse as before.
  const { prefix, tail } = useMemo(
    () => splitStableMarkdown(revealed),
    [revealed],
  );

  return (
    <div
      className={`text-[12px] ${streaming ? "[&>p:has(+_[data-streaming-caret])]:inline" : ""}`}
    >
      {prefix ? (
        <>
          <MarkdownSegment text={prefix} />
          {/* mdast-util-to-hast separates top-level blocks with a newline text
              node. Two parses each produce their own but none at the seam, so
              this restores it and keeps split output byte-identical to a whole
              parse — see text-block.split.test.tsx. */}
          {"\n"}
        </>
      ) : null}
      <MarkdownSegment text={tail} />
      {streaming ? (
        <span
          aria-hidden
          data-streaming-caret
          className="ml-0.5 inline-block h-[0.95em] w-[2px] translate-y-[2px] animate-pulse rounded-full bg-nyx-secondary-400 align-baseline"
        />
      ) : null}
    </div>
  );
});
