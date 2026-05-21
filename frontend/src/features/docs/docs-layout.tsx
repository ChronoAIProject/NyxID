import { useEffect, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Search, Menu, X, Bot, LayoutDashboard, Terminal, type LucideIcon } from "lucide-react";
import { useAuthStore } from "@/stores/auth-store";
import {
  DOCS_TABS,
  DOCS_SHARED,
  sidebarForTab,
  type DocGroup,
  type DocTabId,
} from "./manifest";
import { DocsSearch } from "./docs-search";

const GITHUB_URL = "https://github.com/ChronoAIProject";

// Icon per surface — the quick "what is this about" signal in the switcher.
export const SURFACE_ICONS: Record<DocTabId, LucideIcon> = {
  ai: Bot,
  web: LayoutDashboard,
  cli: Terminal,
};

export interface TocItem {
  readonly id: string;
  readonly text: string;
}

function firstSlug(tabId: DocTabId): string {
  const tab = DOCS_TABS.find((t) => t.id === tabId);
  return tab?.groups[0]?.pages[0]?.slug ?? "";
}

function GithubMark() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-label="GitHub">
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

// The CLI / Web / AI-assisted switcher. Lives at the top of the sidebar, above
// the surface's own groups, mirroring the app sidebar's icon + label nav rows.
function SurfaceSwitcher({
  activeTab,
  onNavigate,
}: {
  readonly activeTab: DocTabId;
  readonly onNavigate?: () => void;
}) {
  return (
    <div className="space-y-0.5">
      {DOCS_TABS.map((tab) => {
        const Icon = SURFACE_ICONS[tab.id];
        const active = tab.id === activeTab;
        return (
          <Link
            key={tab.id}
            to="/docs/$"
            params={{ _splat: firstSlug(tab.id) }}
            onClick={onNavigate}
            className={`flex items-center gap-3 rounded-md px-3 py-2 text-[13px] transition-colors ${
              active
                ? "bg-white/[0.06] font-medium text-foreground"
                : "text-muted-foreground hover:bg-white/[0.03] hover:text-foreground"
            }`}
          >
            <Icon
              className={`h-4 w-4 shrink-0 ${active ? "text-nyx-secondary-400" : ""}`}
              aria-hidden
            />
            {tab.label}
          </Link>
        );
      })}
    </div>
  );
}

function SidebarGroups({
  groups,
  currentSlug,
  onNavigate,
}: {
  readonly groups: readonly DocGroup[];
  readonly currentSlug?: string;
  readonly onNavigate?: () => void;
}) {
  return (
    <>
      {groups.map((group) => (
        <div key={group.group} className="mb-6">
          <p className="mb-2 font-mono text-[11px] tracking-widest text-text-tertiary uppercase">
            {group.group}
          </p>
          <ul className="space-y-0.5">
            {group.pages.map((page) => {
              const active = page.slug === currentSlug;
              return (
                <li key={page.slug}>
                  <Link
                    to="/docs/$"
                    params={{ _splat: page.slug }}
                    onClick={onNavigate}
                    className={`block rounded-md px-3 py-1.5 text-sm transition-colors ${
                      active
                        ? "bg-white/[0.06] font-medium text-foreground"
                        : "text-muted-foreground hover:bg-white/[0.03] hover:text-foreground"
                    }`}
                  >
                    {page.title}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </>
  );
}

function Sidebar({
  activeTab,
  currentSlug,
  onNavigate,
}: {
  readonly activeTab: DocTabId;
  readonly currentSlug?: string;
  readonly onNavigate?: () => void;
}) {
  const { surface } = sidebarForTab(activeTab);
  return (
    <nav aria-label="Docs navigation">
      <SurfaceSwitcher activeTab={activeTab} onNavigate={onNavigate} />
      <div className="mt-6 border-t border-border/60 pt-6">
        <SidebarGroups groups={surface} currentSlug={currentSlug} onNavigate={onNavigate} />
      </div>
      <div className="border-t border-border/60 pt-6">
        <SidebarGroups groups={DOCS_SHARED} currentSlug={currentSlug} onNavigate={onNavigate} />
      </div>
    </nav>
  );
}

export function DocsLayout({
  activeTab,
  currentSlug,
  toc,
  activeTocId,
  hideNav,
  children,
}: {
  readonly activeTab: DocTabId;
  readonly currentSlug?: string;
  readonly toc?: readonly TocItem[];
  readonly activeTocId?: string;
  readonly hideNav?: boolean;
  readonly children: ReactNode;
}) {
  const isAuthed = useAuthStore((s) => s.isAuthenticated);
  const [searchOpen, setSearchOpen] = useState(false);
  const [mobileNav, setMobileNav] = useState(false);

  // `/` opens search (unless typing in a field).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
      if (e.key === "/" && !typing) {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="min-h-dvh bg-background font-sans text-foreground">
      <header className="fixed top-0 right-0 left-0 z-40 border-b border-border bg-background/80 backdrop-blur-md">
        <div className="mx-auto flex h-14 max-w-7xl items-center gap-3 px-4 sm:px-6">
          {!hideNav && (
            <button
              type="button"
              aria-label="Open navigation"
              className="text-muted-foreground hover:text-foreground lg:hidden"
              onClick={() => setMobileNav(true)}
            >
              <Menu className="h-5 w-5" />
            </button>
          )}

          <a href="/" className="flex shrink-0 items-center">
            <img src="/nyxid-wordmark.svg" alt="NyxID" className="h-7 w-auto" />
          </a>
          <span className="text-text-tertiary" aria-hidden>
            /
          </span>
          <Link
            to="/docs"
            className="font-display text-[15px] font-semibold tracking-tight text-foreground"
          >
            Docs
          </Link>

          <div className="ml-auto flex items-center gap-2 sm:gap-3">
            <button
              type="button"
              onClick={() => setSearchOpen(true)}
              className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-sm text-muted-foreground transition-colors hover:border-white/[0.15] hover:text-foreground"
            >
              <Search className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Search</span>
              <kbd className="hidden rounded border border-border px-1 font-mono text-[10px] text-text-tertiary sm:inline">
                /
              </kbd>
            </button>
            <a
              href={GITHUB_URL}
              target="_blank"
              rel="noopener noreferrer"
              aria-label="GitHub"
              className="hidden text-muted-foreground transition-colors hover:text-foreground sm:block"
            >
              <GithubMark />
            </a>
            <a
              href={isAuthed ? "/dashboard" : "/login"}
              className="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-white transition-colors hover:bg-nyx-600"
            >
              {isAuthed ? "Dashboard" : "Log in"}
            </a>
          </div>
        </div>
      </header>

      {hideNav ? (
        <div className="mx-auto max-w-5xl px-4 pt-14 sm:px-6">
          <main className="py-10">{children}</main>
        </div>
      ) : (
        <div className="mx-auto flex max-w-7xl gap-8 px-4 pt-14 sm:px-6">
          <aside className="hidden w-56 shrink-0 lg:block">
            <div className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto py-8 pr-2">
              <Sidebar activeTab={activeTab} currentSlug={currentSlug} />
            </div>
          </aside>

          <main className="min-w-0 flex-1 py-10">{children}</main>

          {toc && toc.length > 0 && (
            <aside className="hidden w-52 shrink-0 xl:block">
              <div className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto py-10">
                <p className="mb-3 font-mono text-[11px] tracking-widest text-text-tertiary uppercase">
                  On this page
                </p>
                <ul className="space-y-2 border-l border-border">
                  {toc.map((item) => {
                    const active = item.id === activeTocId;
                    return (
                      <li key={item.id}>
                        <a
                          href={`#${item.id}`}
                          aria-current={active ? "location" : undefined}
                          className={`-ml-px block border-l pl-3 text-sm transition-colors ${
                            active
                              ? "border-nyx-secondary-400 font-medium text-foreground"
                              : "border-transparent text-muted-foreground hover:border-border hover:text-foreground"
                          }`}
                        >
                          {item.text}
                        </a>
                      </li>
                    );
                  })}
                </ul>
              </div>
            </aside>
          )}
        </div>
      )}

      {/* Mobile nav drawer */}
      {mobileNav && (
        <div className="fixed inset-0 z-50 lg:hidden">
          <div className="absolute inset-0 bg-black/60" onClick={() => setMobileNav(false)} />
          <div className="absolute top-0 left-0 h-full w-72 overflow-y-auto border-r border-border bg-background p-5">
            <div className="mb-5 flex items-center justify-between">
              <span className="font-display text-[15px] font-semibold tracking-tight text-foreground">
                Docs
              </span>
              <button type="button" aria-label="Close" onClick={() => setMobileNav(false)}>
                <X className="h-5 w-5 text-muted-foreground" />
              </button>
            </div>
            <Sidebar
              activeTab={activeTab}
              currentSlug={currentSlug}
              onNavigate={() => setMobileNav(false)}
            />
          </div>
        </div>
      )}

      <DocsSearch open={searchOpen} onClose={() => setSearchOpen(false)} />
    </div>
  );
}
