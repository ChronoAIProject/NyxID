import { useEffect, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import { docTabForSlug } from "../docs/manifest";
import type { DocTabId } from "../docs/manifest";
export function useDocumentNavigation(
  slug: string,
  content: RefObject<HTMLElement | null>,
) {
  const tab = docTabForSlug(slug);
  const lastTab = useSyncExternalStore(
    (listener) => {
      window.addEventListener("storage", listener);
      return () => window.removeEventListener("storage", listener);
    },
    () => {
      try {
        const tab = sessionStorage.getItem("nyxid:docs-tab");
        return tab === "web" || tab === "cli" ? tab : "ai";
      } catch {
        return "ai";
      }
    },
    () => "ai" as DocTabId,
  );
  const [outline, setOutline] = useState<{
    toc: { id: string; text: string }[];
    active?: string;
  }>({ toc: [] });
  useEffect(() => {
    try {
      if (tab !== "shared") sessionStorage.setItem("nyxid:docs-tab", tab);
    } catch {
      /* The default AI navigation stays usable without storage. */
    }
    const headings = Array.from(
      content.current?.querySelectorAll<HTMLElement>("h2[id]") ?? [],
    );
    const toc = headings.map((h) => ({ id: h.id, text: h.textContent ?? "" }));
    const update = () => {
      let active = headings[0]?.id;
      for (const heading of headings) {
        if (heading.getBoundingClientRect().top <= 96) active = heading.id;
        else break;
      }
      if (
        window.innerHeight + window.scrollY >=
        document.documentElement.scrollHeight - 2
      )
        active = headings.at(-1)?.id;
      setOutline({ toc, active });
    };
    update();
    const observer = new IntersectionObserver(update, {
      rootMargin: "-72px 0px 0px 0px",
      threshold: [0, 1],
    });
    headings.forEach((h) => observer.observe(h));
    // The server already rendered headings; scroll again after client takeover.
    try {
      document
        .getElementById(decodeURIComponent(window.location.hash.slice(1)))
        ?.scrollIntoView();
    } catch {
      /* An invalid hash never hides the document. */
    }
    return () => observer.disconnect();
  }, [slug, tab, content]);
  return {
    activeTab: tab === "shared" ? lastTab : tab,
    toc: outline.toc,
    activeTocId: outline.active,
  };
}
