import { createContext, useContext, useEffect, useMemo, useRef, type ReactNode } from "react";

const ReviewCardNavigationContext = createContext<{ selectionKey: string; handled: boolean } | undefined>(undefined);

/** Survives virtualized rows; changing the linked card starts a new reveal. */
export function ReviewCardNavigation({ selectionKey, children }: { readonly selectionKey: string; readonly children: ReactNode }) {
  const navigation = useMemo(() => ({ selectionKey, handled: false }), [selectionKey]);
  return <ReviewCardNavigationContext value={navigation}>{children}</ReviewCardNavigationContext>;
}

export function useReviewCardNavigation() {
  return useContext(ReviewCardNavigationContext);
}

/** History mounts cards after loading, including cards in hidden member threads. */
export function ReviewCardTarget({ selected, selectionKey, children }: {
  readonly selected: boolean;
  readonly selectionKey?: string;
  readonly children: ReactNode;
}) {
  const target = useRef<HTMLDivElement>(null);
  const navigation = useReviewCardNavigation();
  useEffect(() => {
    const element = target.current;
    if (!selected || !element || navigation?.handled) return;
    let timer: number | undefined;
    const frame = requestAnimationFrame(() => {
      if (navigation?.handled) return;
      if (navigation) navigation.handled = true;
      element.scrollIntoView({ block: "center", behavior: "instant" });
      element.classList.add("ring-2", "ring-input-focus");
      timer = window.setTimeout(() => element.classList.remove("ring-2", "ring-input-focus"), 4000);
    });
    return () => {
      cancelAnimationFrame(frame);
      if (timer !== undefined) window.clearTimeout(timer);
      element.classList.remove("ring-2", "ring-input-focus");
    };
  }, [selected, selectionKey, navigation]);
  return <div ref={target} className="scroll-m-4 rounded-xl">{children}</div>;
}
