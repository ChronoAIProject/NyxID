import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";

import { useLogoHref } from "@/hooks/use-logo-href";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Renders a legal document (privacy / terms / etc) by fetching the
 * canonical markdown from `public/legal/<doc>.md`. Same files are served
 * to the mobile app at `${LEGAL_BASE_URL}/legal/<doc>.md` so both surfaces
 * always show the same text without duplicating content.
 *
 * If you need to update the policy: edit the markdown file in
 * `frontend/public/legal/` and redeploy frontend. Mobile picks up the
 * new text on the next launch.
 */
type Props = {
  title: string;
  mdPath: string;
  docKey: string;
};

const FRONT_MATTER_RE = /^---\n([\s\S]*?)\n---\n*/;

function stripFrontMatter(md: string): { content: string; effectiveDate: string | null } {
  const match = md.match(FRONT_MATTER_RE);
  if (!match || !match[0] || !match[1]) return { content: md, effectiveDate: null };
  const body = md.slice(match[0].length);
  const dateMatch = match[1].match(/effective_date:\s*(\S+)/);
  return { content: body, effectiveDate: dateMatch?.[1] ?? null };
}

export function LegalDocumentPage({ title, mdPath, docKey }: Props) {
  const logoHref = useLogoHref();
  const [content, setContent] = useState<string>("");
  const [effectiveDate, setEffectiveDate] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch(mdPath, { cache: "no-store" })
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.text();
      })
      .then((md) => {
        if (cancelled) return;
        const { content: body, effectiveDate: date } = stripFrontMatter(md);
        // Drop the first H1 since we already render `title` in the header.
        const withoutH1 = body.replace(/^#\s+.+\n+/, "");
        setContent(withoutH1);
        setEffectiveDate(date);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : "Failed to load document");
      });
    return () => {
      cancelled = true;
    };
  }, [mdPath]);

  return (
    <div
      className="flex min-h-dvh flex-col items-center bg-background px-4 py-8"
      style={{
        paddingTop: "max(2rem, var(--sat))",
        paddingBottom: "max(2rem, var(--sab))",
      }}
    >
      <div className="w-full max-w-[680px] space-y-8">
        <div className="flex flex-col items-center gap-4">
          <Link to={logoHref} className="flex items-center transition-opacity hover:opacity-80">
            <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-8 w-auto" />
          </Link>
          <h1 className="text-2xl font-bold text-foreground">{title}</h1>
          {effectiveDate && (
            <p className="text-xs text-text-tertiary">Effective date: {effectiveDate}</p>
          )}
        </div>

        <div
          className="legal-prose space-y-6 rounded-xl border border-border bg-card p-6 sm:p-8"
          data-doc-key={docKey}
        >
          {error ? (
            <p className="text-sm text-destructive">
              Failed to load the {title.toLowerCase()}: {error}.{" "}
              <a href={mdPath} className="underline">View source</a>.
            </p>
          ) : content ? (
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                h2: ({ children }) => (
                  <h2 className="text-base font-semibold text-foreground mt-6 first:mt-0">{children}</h2>
                ),
                h3: ({ children }) => (
                  <h3 className="text-sm font-semibold text-foreground mt-4">{children}</h3>
                ),
                p: ({ children }) => (
                  <p className="text-sm leading-relaxed text-muted-foreground">{children}</p>
                ),
                ul: ({ children }) => (
                  <ul className="list-inside list-disc space-y-1 pl-2 text-sm text-muted-foreground">
                    {children}
                  </ul>
                ),
                strong: ({ children }) => (
                  <strong className="text-foreground">{children}</strong>
                ),
                a: ({ href, children }) => (
                  <a
                    href={href}
                    className="text-violet-400 underline hover:text-violet-300"
                  >
                    {children}
                  </a>
                ),
                code: ({ children }) => (
                  <code className="font-mono text-xs text-foreground">{children}</code>
                ),
              }}
            >
              {content}
            </ReactMarkdown>
          ) : (
            <p className="text-sm text-muted-foreground">Loading…</p>
          )}
        </div>

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
