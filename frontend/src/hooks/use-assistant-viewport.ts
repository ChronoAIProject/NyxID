import { useLayoutEffect, useRef } from "react";

export function useAssistantViewport() {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const viewport = window.visualViewport;
    const roots = [document.documentElement, document.body];
    const previous = roots.map((root) => ({
      overflow: root.style.overflow,
      overscrollBehavior: root.style.overscrollBehavior,
    }));
    for (const root of roots) {
      root.style.overflow = "hidden";
      root.style.overscrollBehavior = "none";
    }

    function sync() {
      // Keep the existing layout during pinch zoom so magnification can pan.
      if (!element || !viewport || viewport.scale !== 1) return;
      element.style.height = `${viewport.height}px`;
      element.style.top = `${viewport.offsetTop}px`;
    }
    sync();
    viewport?.addEventListener("resize", sync);
    viewport?.addEventListener("scroll", sync);
    window.addEventListener("resize", sync);
    return () => {
      viewport?.removeEventListener("resize", sync);
      viewport?.removeEventListener("scroll", sync);
      window.removeEventListener("resize", sync);
      roots.forEach((root, index) => {
        root.style.overflow = previous[index]!.overflow;
        root.style.overscrollBehavior = previous[index]!.overscrollBehavior;
      });
    };
  }, []);

  return ref;
}
