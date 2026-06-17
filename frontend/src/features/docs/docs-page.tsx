import { useEffect, useRef, useState } from "react";
import { useParams } from "@tanstack/react-router";
import { DocsLayout, type TocItem } from "./docs-layout";
import { DocsIndexPage } from "./docs-index-page";
import { DocMarkdown } from "./docs-markdown";
import { findDocPage, docTabForSlug, type DocTabId } from "./manifest";

const FRONT_MATTER_RE = /^---\n([\s\S]*?)\n---\n*/;

interface ParsedDoc {
  readonly title: string;
  readonly description: string;
  readonly body: string;
}

interface LoadedDoc {
  readonly slug: string;
  readonly status: "ok" | "missing";
  readonly doc: ParsedDoc | null;
}

function parseDoc(md: string): ParsedDoc {
  let title = "";
  let description = "";
  let body = md;
  const m = md.match(FRONT_MATTER_RE);
  if (m) {
    const fm = m[1] ?? "";
    title = fm.match(/title:\s*"?(.+?)"?\s*$/m)?.[1]?.trim() ?? "";
    description = fm.match(/description:\s*"?(.+?)"?\s*$/m)?.[1]?.trim() ?? "";
    body = md.slice(m[0].length);
  }
  // Drop a leading H1 if the author included one — the title renders separately.
  body = body.replace(/^#\s+.+\n+/, "");
  return { title, description, body };
}

function readLastTab(): DocTabId {
  try {
    const last = sessionStorage.getItem("nyxid:docs-tab");
    if (last === "ai" || last === "web" || last === "cli") return last;
  } catch {
    /* sessionStorage unavailable */
  }
  return "ai";
}

type Status = "loading" | "ok" | "missing";

export function DocsPage() {
  const { _splat } = useParams({ from: "/docs/$" });
  const slug = (_splat ?? "").replace(/\/+$/, "");
  const known = findDocPage(slug);

  // Fetched content is tagged with the slug it belongs to and only ever written
  // from async callbacks — never synchronously inside the effect — so status/doc
  // derive cleanly from render without cascading setState.
  const [loaded, setLoaded] = useState<LoadedDoc | null>(null);
  const [toc, setToc] = useState<TocItem[]>([]);
  const [activeId, setActiveId] = useState<string | null>(() => {
    if (typeof window !== "undefined") {
      return decodeURIComponent(window.location.hash.replace(/^#/, "")) || null;
    }
    return null;
  });
  const contentRef = useRef<HTMLDivElement>(null);
  // Tracks the slug we've already handled an incoming `#hash` for, so the
  // load-time deep-link scroll fires exactly once per page.
  const deepLinkedRef = useRef<string | null>(null);

  useEffect(() => {
    if (!known) return;
    let cancelled = false;
    fetch(`/docs/${slug}.md`, { cache: "no-store" })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status));
        return r.text();
      })
      .then((md) => {
        if (!cancelled) setLoaded({ slug, status: "ok", doc: parseDoc(md) });
      })
      .catch(() => {
        if (!cancelled) setLoaded({ slug, status: "missing", doc: null });
      });
    return () => {
      cancelled = true;
    };
  }, [slug, known]);

  // Reset activeId when transitioning to a new page (adjust state during render to avoid cascading effects)
  const [prevSlug, setPrevSlug] = useState<string>(slug);
  if (slug !== prevSlug) {
    setPrevSlug(slug);
    setActiveId(decodeURIComponent(window.location.hash.replace(/^#/, "")) || null);
  }

  // Unknown slugs are a 404 immediately; otherwise show the fetched doc once it
  // matches the current slug, and "loading" until it catches up.
  const status: Status = !known
    ? "missing"
    : loaded?.slug === slug
      ? loaded.status
      : "loading";
  const doc = known && loaded?.slug === slug ? loaded.doc : null;

  // Build the on-page TOC from rendered h2 headings (ids come from rehype-slug).
  useEffect(() => {
    if (status !== "ok") return;
    const root = contentRef.current;
    if (!root) return;
    const items: TocItem[] = Array.from(
      root.querySelectorAll<HTMLHeadingElement>("h2[id]"),
    ).map((h) => ({ id: h.id, text: h.textContent ?? "" }));
    setToc(items);
  }, [status, doc]);

  // Honor an incoming `#section` deep link. The markdown is fetched async, so
  // the browser's native on-load anchor scroll fires before the heading exists
  // and silently misses — we re-run it once the content is in the DOM. Declared
  // before the scroll-spy effect so the scroll lands before scroll-spy reads
  // positions (otherwise it would clear the hash as "still at the top").
  useEffect(() => {
    if (status !== "ok") return;
    const root = contentRef.current;
    if (!root || deepLinkedRef.current === slug) return;
    deepLinkedRef.current = slug;
    const id = decodeURIComponent(window.location.hash.replace(/^#/, ""));
    if (!id) return;
    // Only move the viewport — the scroll-spy effect (declared below) reads the
    // post-scroll position and sets the highlight, keeping setState out of here.
    root.querySelector<HTMLElement>(`#${CSS.escape(id)}`)?.scrollIntoView();
  }, [status, slug]);

  // The URL hash is the source of truth for the highlight: clicking a TOC link
  // (or back/forward between sections) updates `#hash`, which we mirror into the
  // active id. Scroll-driven hash updates use replaceState and don't fire this.
  useEffect(() => {
    const onHash = () => {
      const id = decodeURIComponent(window.location.hash.replace(/^#/, ""));
      if (id) setActiveId(id);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // Scroll-spy: highlight the TOC entry for the section currently being read.
  // Active = the last h2 whose top has scrolled above a trigger line just below
  // the fixed header. An IntersectionObserver fires the (cheap) recompute only
  // when a heading crosses that line, rather than on every scroll frame. The
  // 96px trigger matches the headings' `scroll-mt-24`, so clicking a TOC link
  // lands its section as the active one too.
  useEffect(() => {
    if (status !== "ok") return;
    const root = contentRef.current;
    if (!root) return;
    const headings = Array.from(root.querySelectorAll<HTMLElement>("h2[id]"));
    if (headings.length === 0) return;

    const TRIGGER = 96;
    const recompute = () => {
      let current: string | null = null;
      for (const h of headings) {
        if (h.getBoundingClientRect().top - TRIGGER <= 1) current = h.id;
        else break;
      }
      // At the bottom of the page the last section can't reach the trigger line,
      // so short trailing sections would never highlight — pin to the last
      // heading once scrolled to the end (also fixes deep-links to them).
      const atBottom =
        window.innerHeight + window.scrollY >= document.documentElement.scrollHeight - 2;
      const next = atBottom ? (headings[headings.length - 1]?.id ?? null) : (current ?? headings[0]?.id ?? null);
      setActiveId(next);
    };
    recompute();
    const observer = new IntersectionObserver(recompute, {
      rootMargin: "-72px 0px 0px 0px",
      threshold: [0, 1],
    });
    headings.forEach((h) => observer.observe(h));
    return () => observer.disconnect();
  }, [status, toc]);

  // Remember the last real surface so concept pages keep the right tab active.
  const rawTab = docTabForSlug(slug);
  useEffect(() => {
    if (rawTab !== "shared") {
      try {
        sessionStorage.setItem("nyxid:docs-tab", rawTab);
      } catch {
        /* ignore */
      }
    }
  }, [rawTab]);

  // Title + description + canonical + Open Graph/Twitter tags for SEO & sharing.
  // Tags we create are removed on unmount; tags we only edit are restored.
  useEffect(() => {
    if (status !== "ok" || !doc) return;
    const fullTitle = `${doc.title || known?.title || "Docs"} · NyxID Docs`;
    const description = doc.description;
    const url = `${window.location.origin}/docs/${slug}`;

    const prevTitle = document.title;
    document.title = fullTitle;

    const cleanups: Array<() => void> = [];
    const upsert = (
      selector: string,
      create: () => HTMLElement,
      attr: string,
      value: string,
    ) => {
      if (!value) return;
      const existing = document.head.querySelector<HTMLElement>(selector);
      if (existing) {
        const prev = existing.getAttribute(attr);
        existing.setAttribute(attr, value);
        cleanups.push(() => {
          if (prev !== null) existing.setAttribute(attr, prev);
        });
      } else {
        const el = create();
        el.setAttribute(attr, value);
        document.head.appendChild(el);
        cleanups.push(() => el.remove());
      }
    };
    const meta = (name: string) => () => {
      const m = document.createElement("meta");
      m.setAttribute("name", name);
      return m;
    };
    const ogMeta = (property: string) => () => {
      const m = document.createElement("meta");
      m.setAttribute("property", property);
      return m;
    };

    upsert('meta[name="description"]', meta("description"), "content", description);
    upsert(
      'link[rel="canonical"]',
      () => {
        const l = document.createElement("link");
        l.setAttribute("rel", "canonical");
        return l;
      },
      "href",
      url,
    );
    upsert('meta[property="og:type"]', ogMeta("og:type"), "content", "article");
    upsert('meta[property="og:title"]', ogMeta("og:title"), "content", fullTitle);
    upsert('meta[property="og:description"]', ogMeta("og:description"), "content", description);
    upsert('meta[property="og:url"]', ogMeta("og:url"), "content", url);
    upsert('meta[name="twitter:card"]', meta("twitter:card"), "content", "summary");

    return () => {
      document.title = prevTitle;
      for (const cleanup of cleanups) cleanup();
    };
  }, [status, doc, known, slug]);

  const activeTab: DocTabId = rawTab === "shared" ? readLastTab() : rawTab;
  const baseDir = slug.includes("/") ? slug.slice(0, slug.lastIndexOf("/")) : "";
  const baseHref = `/docs/${baseDir ? baseDir + "/" : ""}`;
  const title = doc?.title || known?.title || "";

  // The `/docs/$` splat route also matches the bare `/docs` path (empty splat),
  // out-ranking the index route on direct loads. Render the landing there.
  if (!slug) return <DocsIndexPage />;

  return (
    <DocsLayout
      activeTab={activeTab}
      currentSlug={slug}
      toc={status === "ok" ? toc : []}
      activeTocId={activeId ?? undefined}
    >
      {status === "loading" && (
        <div className="max-w-3xl animate-pulse space-y-4" aria-busy="true">
          <div className="h-9 w-2/3 rounded bg-overlay-strong" />
          <div className="h-4 w-full rounded bg-overlay" />
          <div className="h-4 w-5/6 rounded bg-overlay" />
          <div className="h-4 w-4/6 rounded bg-overlay" />
        </div>
      )}
      {status === "missing" && (
        <div className="max-w-3xl rounded-2xl border border-border bg-card p-10 text-center">
          <p className="font-mono text-xs tracking-widest text-text-tertiary uppercase">404</p>
          <h1 className="mt-3 font-display text-2xl font-semibold tracking-tight text-foreground">Page not found</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            This docs page doesn’t exist (yet). Try search, or pick a section from the sidebar.
          </p>
        </div>
      )}
      {status === "ok" && doc && (
        <article ref={contentRef} className="max-w-3xl">
          <h1 className="mb-6 font-display text-3xl font-bold tracking-tight text-foreground md:text-4xl">{title}</h1>
          <DocMarkdown markdown={doc.body} baseHref={baseHref} />
        </article>
      )}
    </DocsLayout>
  );
}
