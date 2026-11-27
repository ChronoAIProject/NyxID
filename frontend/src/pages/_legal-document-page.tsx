import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";

import { useApplyTheme } from "@/hooks/use-theme";
import { PublicFooter, PublicHeader } from "@/features/public-site/public-chrome";
import { usePublicThemeElement } from "@/features/public-site/public-theme";
import "@/features/website-a/public-reading.css";
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
  const { i18n } = useUiTranslation();
  // The app's text size and density still apply; colours follow the public
  // day/night choice, which never changes the app's own theme preference.
  useApplyTheme();
  const scope = useRef<HTMLDivElement>(null);
  usePublicThemeElement(scope);
  const language = i18n.resolvedLanguage?.startsWith("zh") ? "zh" : "en";
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
        // The canonical raw document includes a readable date for Markdown
        // consumers. Render the metadata date only when the body lacks it.
        setEffectiveDate(date && body.includes(`**Effective date: ${date}**`) ? null : date);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : "Failed to load document");
      });
    return () => {
      cancelled = true;
    };
  }, [mdPath]);

  // Reading sizes, rules and the prose width live in public-reading.css (.public-legal).
  return (
    <div
      ref={scope}
      className="nyx-public public-reading public-legal flex flex-col"
      style={{ paddingBottom: "max(0px, var(--sab))" }}
    >
      <PublicHeader language={language} />
      <main className="public-legal-main public-wrap flex-1">
        <header className="public-legal-head">
          <h1 className="text-foreground">{title}</h1>
          {effectiveDate && (
            <p className="public-legal-date">{translateUi("Effective date: ")}{effectiveDate}</p>
          )}
        </header>

        <div
          className="legal-prose text-foreground space-y-6 bg-card"
          data-doc-key={docKey}
        >
          {error ? (
            <p className="text-destructive">{translateUi("Failed to load the ")}{title.toLowerCase()}: {translateUi(error)}.{" "}
              <a href={mdPath} className="underline">{translateUi("View source")}</a>.
            </p>
          ) : content ? (
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                h2: ({ children }) => (
                  <h2 className="text-foreground">{children}</h2>
                ),
                h3: ({ children }) => (
                  <h3 className="text-foreground">{children}</h3>
                ),
                p: ({ children }) => (
                  <p className="text-muted-foreground">{children}</p>
                ),
                ul: ({ children }) => (
                  <ul className="list-outside list-disc space-y-2 pl-5 text-muted-foreground">
                    {children}
                  </ul>
                ),
                strong: ({ children }) => (
                  <strong className="text-foreground">{children}</strong>
                ),
                a: ({ href, children }) => (
                  <a
                    href={href}
                    className="text-foreground underline hover:text-muted-foreground"
                  >
                    {children}
                  </a>
                ),
                code: ({ children }) => (
                  <code className="font-mono text-foreground">{children}</code>
                ),
              }}
            >
              {content}
            </ReactMarkdown>
          ) : (
            <p className="text-muted-foreground">{translateUi("Loading…")}</p>
          )}
        </div>

        <Link
          to="/"
          className="public-legal-back text-foreground underline transition-colors hover:text-muted-foreground"
        >
          <ArrowLeft className="h-3 w-3" aria-hidden />{translateUi("Back to NyxID")}</Link>
      </main>
      <PublicFooter language={language} showStatus={false} />
    </div>
  );
}
