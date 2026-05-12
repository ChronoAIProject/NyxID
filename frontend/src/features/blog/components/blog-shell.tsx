import { useState } from "react";

import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";
import { LandingFooter } from "@/features/landing/components/landing-footer";
import { useScroll } from "@/features/landing/hooks/use-scroll";
import "@/features/landing/landing.css";

// Visually matches LandingNavbar (border, glow, blur) but uses absolute hrefs
// so cross-page links work from /blog and /preview without scrolling to a
// non-existent local anchor.
function BlogNavbar() {
  const [scrolled, setScrolled] = useState(false);
  useScroll((scrollY) => setScrolled(scrollY > 0));

  return (
    <nav
      className={`fixed top-0 right-0 left-0 z-50 border-b backdrop-blur-md transition-all duration-500 ${
        scrolled
          ? "navbar-glow border-primary/20 bg-landing-bg/80"
          : "border-landing-border-subtle bg-landing-bg/80"
      }`}
    >
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <a href="/" className="flex items-center gap-2">
          <PortalMarkLogo size={32} />
          <span className="font-serif text-xl text-white">NyxID</span>
        </a>

        <div className="flex items-center gap-3">
          <a
            href="/blog"
            className="hidden text-sm font-medium text-gray-300 transition-colors hover:text-white md:inline-block"
          >
            Field Notes
          </a>
          <a
            href="/login"
            className="rounded-lg border border-primary/40 px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary/10"
          >
            Login
          </a>
          <a
            href="/#beta"
            className="hidden rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-white transition-colors hover:bg-nyx-400 md:inline-block"
          >
            Request Beta
          </a>
        </div>
      </div>
    </nav>
  );
}

export function BlogShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-landing-bg min-h-screen overflow-x-hidden">
      <BlogNavbar />
      <main className="pt-16">{children}</main>
      <LandingFooter />
    </div>
  );
}
