import { useLayoutEffect, useSyncExternalStore, type RefObject } from "react";
import { flushSync } from "react-dom";

export type PublicTheme = "light" | "dark";
const THEME_KEY = "nyxid-theme";
const CHANGE_EVENT = "nyxid:public-theme";

function savedTheme(): PublicTheme | null {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    return saved === "light" || saved === "dark" ? saved : null;
  } catch {
    return null;
  }
}

export function resolvePublicTheme(): PublicTheme {
  return savedTheme() ??
    (typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
}

function subscribe(listener: () => void) {
  const media = typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  window.addEventListener(CHANGE_EVENT, listener);
  window.addEventListener("storage", listener);
  media?.addEventListener("change", listener);
  return () => {
    window.removeEventListener(CHANGE_EVENT, listener);
    window.removeEventListener("storage", listener);
    media?.removeEventListener("change", listener);
  };
}

let visitTheme: PublicTheme | null = null;
const snapshot = () => visitTheme ?? resolvePublicTheme();
const serverSnapshot = (): PublicTheme | null => null;

/** Public pages share one preference without changing the signed-in application's theme. */
export function usePublicTheme() {
  const theme = useSyncExternalStore(subscribe, snapshot, serverSnapshot);
  return {
    theme,
    toggleTheme: () => {
      const next = snapshot() === "dark" ? "light" : "dark";
      const change = () => {
        try {
          localStorage.setItem(THEME_KEY, next);
          visitTheme = null;
        } catch {
          visitTheme = next;
        }
        window.dispatchEvent(new Event(CHANGE_EVENT));
      };
      if (document.startViewTransition && !(typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches))
        document.startViewTransition(() => flushSync(change));
      else change();
    },
  };
}

/** Apply before paint; leave html classes, app tokens and app preferences alone. */
export function usePublicThemeElement(ref: RefObject<HTMLElement | null>) {
  const { theme } = usePublicTheme();
  useLayoutEffect(() => {
    if (!ref.current) return;
    const current = snapshot();
    if (ref.current.dataset.theme !== current) ref.current.dataset.theme = current;
    const meta = document.querySelector('meta[name="theme-color"]');
    const previous = meta?.getAttribute("content");
    meta?.setAttribute("content", current === "dark" ? "#0E0E10" : "#F3F3F0");
    return () => {
      if (previous == null) meta?.removeAttribute("content");
      else meta?.setAttribute("content", previous);
    };
  });
  return theme;
}
