import { useEffect, useRef, type ReactNode } from "react";

/** History mounts cards after loading, including cards in hidden member threads. */
export function ReviewCardTarget({ selected, children }: {
  readonly selected: boolean;
  readonly children: ReactNode;
}) {
  const target = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = target.current;
    if (!selected || !element) return;
    let timer: number | undefined;
    const frame = requestAnimationFrame(() => {
      element.scrollIntoView({ block: "center", behavior: "instant" });
      element.classList.add("ring-2", "ring-input-focus");
      timer = window.setTimeout(() => element.classList.remove("ring-2", "ring-input-focus"), 4000);
    });
    return () => {
      cancelAnimationFrame(frame);
      if (timer !== undefined) window.clearTimeout(timer);
      element.classList.remove("ring-2", "ring-input-focus");
    };
  }, [selected]);
  return <div ref={target} className="scroll-m-4 rounded-xl">{children}</div>;
}
