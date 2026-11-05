import { useEffect, useRef, useState } from "react";

export function usePanelVisibility() {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(
    () => typeof IntersectionObserver === "undefined",
  );
  const [hasEntered, setHasEntered] = useState(visible);
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        // Font/layout changes can queue several records before delivery.
        const next = entries.at(-1)?.isIntersecting ?? false;
        setVisible(next);
        if (entries.some((entry) => entry.isIntersecting)) setHasEntered(true);
      },
      { rootMargin: "240px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return { ref, visible, hasEntered };
}
