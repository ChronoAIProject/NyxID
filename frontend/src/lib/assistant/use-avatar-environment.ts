import {
  useEffect,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";
import { useThemeStore } from "@/stores/theme-store";
import type { AvatarTheme } from "@/lib/assistant/agent-avatar-model";

// The applied theme lives on <html> (`theme-dark` / `theme-light`). One
// observer serves every avatar on the page.
const themeListeners = new Set<() => void>();
let themeObserver: MutationObserver | undefined;

function documentTheme(): AvatarTheme {
  if (typeof document === "undefined") return "dark";
  const root = document.documentElement.classList;
  if (root.contains("theme-light")) return "light";
  if (root.contains("theme-dark")) return "dark";
  // Unthemed pages keep the dark defaults.
  return "dark";
}

function subscribeTheme(listener: () => void): () => void {
  themeListeners.add(listener);
  if (
    !themeObserver &&
    typeof MutationObserver !== "undefined" &&
    typeof document !== "undefined"
  ) {
    themeObserver = new MutationObserver(() =>
      themeListeners.forEach((notify) => notify()),
    );
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
  }
  return () => {
    themeListeners.delete(listener);
    if (themeListeners.size === 0) {
      themeObserver?.disconnect();
      themeObserver = undefined;
    }
  };
}

/** The theme avatars paint for (light or dark palette variants). */
export function useDocumentTheme(): AvatarTheme {
  return useSyncExternalStore(subscribeTheme, documentTheme, documentTheme);
}

const REDUCE_QUERY = "(prefers-reduced-motion: reduce)";

function subscribeReduce(listener: () => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia)
    return () => undefined;
  const query = window.matchMedia(REDUCE_QUERY);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}

function systemReduces(): boolean {
  return (
    typeof window !== "undefined" &&
    Boolean(window.matchMedia?.(REDUCE_QUERY).matches)
  );
}

/** System reduced motion or Settings → Display reduced motion. */
export function useReducedAvatarMotion(): boolean {
  const preference = useThemeStore((state) => state.motion);
  const system = useSyncExternalStore(
    subscribeReduce,
    systemReduces,
    () => false,
  );
  return preference === "reduce" || system;
}

function subscribeVisibility(listener: () => void): () => void {
  if (typeof document === "undefined") return () => undefined;
  document.addEventListener("visibilitychange", listener);
  window.addEventListener("focus", listener);
  window.addEventListener("blur", listener);
  return () => {
    document.removeEventListener("visibilitychange", listener);
    window.removeEventListener("focus", listener);
    window.removeEventListener("blur", listener);
  };
}

function pageVisible(): boolean {
  return (
    typeof document === "undefined" ||
    (document.visibilityState !== "hidden" && document.hasFocus())
  );
}

/** Animate only while the element is on screen and the tab is visible. */
export function useAnimationActive(ref: RefObject<Element | null>): boolean {
  const visible = useSyncExternalStore(
    subscribeVisibility,
    pageVisible,
    () => true,
  );
  const [onScreen, setOnScreen] = useState(
    typeof IntersectionObserver === "undefined",
  );
  useEffect(() => {
    const element = ref.current;
    if (!element || typeof IntersectionObserver === "undefined") return;
    let intersecting = false;
    const update = () =>
      setOnScreen(
        intersecting &&
          !element.closest("[hidden], [inert], [data-aria-hidden=true]"),
      );
    const observer = new IntersectionObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (entry) {
        intersecting = entry.isIntersecting;
        update();
      }
    });
    const covers = new MutationObserver(update);
    covers.observe(document.documentElement, {
      subtree: true,
      attributes: true,
      attributeFilter: ["hidden", "inert", "data-aria-hidden"],
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      covers.disconnect();
    };
  }, [ref]);
  return visible && onScreen;
}
