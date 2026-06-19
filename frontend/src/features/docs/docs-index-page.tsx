import { useEffect } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { DocsLayout, SURFACE_ICONS } from "./docs-layout";
import { DOCS_TABS, DOCS_SHARED } from "./manifest";

export function DocsIndexPage() {
  useEffect(() => {
    const prev = document.title;
    document.title = "nyxid - docs";
    return () => {
      document.title = prev;
    };
  }, []);

  return (
    <DocsLayout activeTab="ai" hideNav>
      <div className="py-6">
        <p className="font-mono text-xs tracking-widest text-nyx-700 dark:text-nyx-secondary-400 uppercase">NyxID Documentation</p>
        <h1 className="mt-3 max-w-3xl font-display text-4xl font-bold leading-tight tracking-tight text-foreground md:text-5xl">
          Broker credentials. Never expose keys.
        </h1>
        <p className="mt-4 max-w-2xl text-lg text-muted-foreground">
          NyxID holds your API keys, OAuth tokens, and SSH credentials and proxies requests so your
          agents and apps never touch the raw secret. Pick how you use it.
        </p>

        <div className="mt-10 grid gap-4 sm:grid-cols-3">
          {DOCS_TABS.map((tab, i) => {
            const first = tab.groups[0]?.pages[0]?.slug ?? "";
            const Icon = SURFACE_ICONS[tab.id];
            return (
              <Link
                key={tab.id}
                to="/docs/$"
                params={{ _splat: first }}
                className="group rounded-2xl border border-border bg-card p-5 transition-colors hover:border-hairline-strong"
              >
                <div className="flex items-center justify-between">
                  <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-border bg-overlay">
                    <Icon className="h-5 w-5 text-nyx-700 dark:text-nyx-secondary-400" aria-hidden />
                  </div>
                  {i === 0 && (
                    <span className="rounded-full bg-nyx-100 px-2 py-0.5 text-[10px] font-medium text-nyx-700 dark:bg-nyx-500/15 dark:text-nyx-secondary-400">
                      Start here
                    </span>
                  )}
                </div>
                <h2 className="mt-4 font-display text-xl font-semibold tracking-tight text-foreground">{tab.label}</h2>
                <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{tab.blurb}</p>
                <span className="mt-4 inline-flex items-center gap-1 text-sm font-medium text-nyx-700 dark:text-nyx-secondary-400">
                  Get started
                  <ArrowRight className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5" />
                </span>
              </Link>
            );
          })}
        </div>

        <div className="mt-12">
          <p className="mb-3 font-mono text-[11px] tracking-widest text-text-tertiary uppercase">Concepts</p>
          <div className="grid gap-2 sm:grid-cols-2">
            {DOCS_SHARED[0]?.pages.map((p) => (
              <Link
                key={p.slug}
                to="/docs/$"
                params={{ _splat: p.slug }}
                className="rounded-lg border border-border px-4 py-2.5 text-sm text-muted-foreground transition-colors hover:border-hairline-strong hover:text-foreground"
              >
                {p.title}
              </Link>
            ))}
          </div>
        </div>
      </div>
    </DocsLayout>
  );
}
