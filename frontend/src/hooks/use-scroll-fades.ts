import { useEffect, type RefObject } from "react";

/** Adds edge fades to marked scroll regions without changing their layout or scroll ownership. */
export function useScrollFades(rootRef: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const regions = new Set<HTMLElement>();
    const observed = new Set<Element>();
    let frame = 0;
    let discover = true;

    function update(element: HTMLElement) {
      const edges = {
        top: element.scrollTop > 1,
        bottom: element.scrollHeight - element.clientHeight - element.scrollTop > 1,
        left: element.scrollLeft > 1,
        right: element.scrollWidth - element.clientWidth - element.scrollLeft > 1,
      };
      const value = Object.entries(edges).filter(([, hidden]) => hidden).map(([edge]) => edge).join(" ");
      if (element.dataset.scrollEdges !== value) element.dataset.scrollEdges = value;
    }
    function schedule() {
      if (!frame) frame = requestAnimationFrame(refresh);
    }
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    function refresh() {
      frame = 0;
      if (discover) {
        discover = false;
        regions.clear();
        root!.querySelectorAll<HTMLElement>("[data-scroll-fade]").forEach((element) => regions.add(element));
        const nextObserved = new Set<Element>();
        regions.forEach((element) => {
          nextObserved.add(element);
          // Inner content resizing includes late images, fonts, and reflow while
          // the scroll viewport itself remains the same size.
          for (const child of element.children) nextObserved.add(child);
        });
        observed.forEach((element) => {
          if (!nextObserved.has(element)) { resize?.unobserve(element); observed.delete(element); }
        });
        nextObserved.forEach((element) => {
          if (!observed.has(element)) { resize?.observe(element); observed.add(element); }
        });
      }
      regions.forEach(update);
    }
    const mutation = new MutationObserver((records) => {
      if (records.some((record) => record.type === "childList")) discover = true;
      schedule();
    });
    mutation.observe(root, { subtree: true, childList: true, characterData: true });
    function scroll(event: Event) {
      if (event.target instanceof HTMLElement && regions.has(event.target)) update(event.target);
    }
    root.addEventListener("scroll", scroll, { capture: true, passive: true });
    window.addEventListener("resize", schedule);
    refresh();
    return () => {
      cancelAnimationFrame(frame);
      resize?.disconnect();
      mutation.disconnect();
      root.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", schedule);
    };
  }, [rootRef]);
}
