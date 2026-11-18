import { useCallback, useSyncExternalStore } from "react";
import { computedTextSize } from "@/lib/typography";

function subscribe(update: () => void) {
  const observer = new MutationObserver(update);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["style", "class"] });
  window.addEventListener("resize", update);
  return () => { observer.disconnect(); window.removeEventListener("resize", update); };
}

/** For chart geometry APIs that require numbers rather than CSS variables. */
export function useTextSize(token: number) {
  return useSyncExternalStore(subscribe, useCallback(() => computedTextSize(token), [token]), () => token);
}
