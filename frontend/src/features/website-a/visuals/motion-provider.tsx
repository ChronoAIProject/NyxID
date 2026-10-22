import { useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { visualConfig } from "./config";
import { MotionContext } from "./motion";
function subscribe(changed: () => void) {
  const media = window.matchMedia("(prefers-reduced-motion: reduce)");
  media.addEventListener("change", changed);
  document.addEventListener("visibilitychange", changed);
  return () => {
    media.removeEventListener("change", changed);
    document.removeEventListener("visibilitychange", changed);
  };
}
function snapshot() {
  return !visualConfig.enabled ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ? "static"
    : document.hidden
      ? "background"
      : "active";
}
export function VisualMotionProvider({ children }: { children: ReactNode }) {
  const mode = useSyncExternalStore(subscribe, snapshot, () => "static");
  return (
    <MotionContext.Provider value={mode}>{children}</MotionContext.Provider>
  );
}
