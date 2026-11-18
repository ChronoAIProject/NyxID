import { useLayoutEffect, useState } from "react";

/** Observe the editing surface, so a narrow dialog never follows a wide viewport. */
export function useSectionLayout() {
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  const [wide, setWide] = useState<boolean>();
  useLayoutEffect(() => {
    if (!element) return;
    const measure = () => setWide(element.getBoundingClientRect().width >= 672);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return [setElement, wide] as const;
}
