import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";

// Single source of truth for the Terms of Use lives in /docs/legal/.
// `?raw` inlines the markdown at build time so the rendered page can never
// drift from the authored document.
import termsMarkdown from "../../../docs/legal/TERMS_OF_USE_DRAFT.md?raw";

// ── Last-updated date (kept in sync with the markdown header) ──
const LAST_UPDATED = "11 May 2026";

// The markdown's first-line H1 ("# NyxID — Terms of Use") and "Last updated"
// line are rendered by the page header above the body card, so we strip them
// from the markdown body before handing it to ReactMarkdown.
const bodyMarkdown = termsMarkdown
  .replace(/^# NyxID — Terms of Use\s*\n/, "")
  .replace(/^\*\*Last updated:\*\*[^\n]*\n/m, "")
  .trimStart();

const COMPONENTS: Components = {
  h1: ({ children }) => (
    <h2 className="mt-6 mb-3 text-base font-semibold text-foreground">
      {children}
    </h2>
  ),
  h2: ({ children }) => (
    <h2 className="mt-8 mb-3 text-base font-semibold text-foreground">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="mt-5 mb-2 text-sm font-medium text-foreground">
      {children}
    </h3>
  ),
  p: ({ children }) => (
    <p className="mb-3 text-sm leading-relaxed text-muted-foreground">
      {children}
    </p>
  ),
  ul: ({ children }) => (
    <ul className="mb-3 list-disc space-y-1.5 pl-5 text-sm leading-relaxed text-muted-foreground marker:text-muted-foreground/50">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-3 list-decimal space-y-1.5 pl-5 text-sm leading-relaxed text-muted-foreground">
      {children}
    </ol>
  ),
  li: ({ children }) => <li>{children}</li>,
  strong: ({ children }) => (
    <strong className="font-semibold text-foreground">{children}</strong>
  ),
  em: ({ children }) => <em className="italic">{children}</em>,
  a: ({ href, children }) => (
    <a
      href={href}
      target={href?.startsWith("http") ? "_blank" : undefined}
      rel={href?.startsWith("http") ? "noopener noreferrer" : undefined}
      className="text-violet-400 underline decoration-violet-400/40 underline-offset-4 transition-colors hover:text-violet-300 hover:decoration-violet-300"
    >
      {children}
    </a>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-4 rounded-lg border-l-2 border-violet-400/40 bg-muted/30 px-4 py-3 text-sm leading-relaxed text-foreground">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="my-6 border-border" />,
  code: ({ children }) => (
    <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">
      {children}
    </code>
  ),
  table: ({ children }) => (
    <div className="my-4 overflow-x-auto rounded-lg border border-border">
      <table className="w-full text-left text-xs text-muted-foreground">
        {children}
      </table>
    </div>
  ),
  thead: ({ children }) => (
    <thead className="border-b border-border bg-muted/30 text-foreground">
      {children}
    </thead>
  ),
  tr: ({ children }) => (
    <tr className="border-b border-border/60 last:border-0">{children}</tr>
  ),
  th: ({ children }) => <th className="px-3 py-2 font-medium">{children}</th>,
  td: ({ children }) => <td className="px-3 py-2">{children}</td>,
};

export function TermsPage() {
  return (
    <div
      className="flex min-h-dvh flex-col items-center bg-background px-4 py-8"
      style={{
        paddingTop: "max(2rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className="w-full max-w-[980px] space-y-8">
        {/* ── Header ── */}
        <div className="flex flex-col items-center gap-4">
          <Link
            to="/"
            className="flex items-center transition-opacity hover:opacity-80"
          >
            <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-8 w-auto" />
          </Link>
          <h1 className="text-2xl font-bold text-foreground">Terms of Use</h1>
          <p className="text-xs text-text-tertiary">
            Last updated: {LAST_UPDATED}
          </p>
        </div>

        {/* ── Body ── */}
        <div className="rounded-xl border border-border bg-card p-6 sm:p-8">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            rehypePlugins={[rehypeSanitize]}
            components={COMPONENTS}
          >
            {bodyMarkdown}
          </ReactMarkdown>
        </div>

        {/* ── Back link ── */}
        <div className="flex justify-center">
          <Link
            to="/"
            className="flex items-center gap-1.5 text-xs text-violet-400 transition-colors hover:text-violet-300"
          >
            <ArrowLeft className="h-3 w-3" />
            Back to NyxID
          </Link>
        </div>
      </div>
    </div>
  );
}
