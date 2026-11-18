import { useEffect } from "react";

const APPLE = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** The keys shown for the search shortcut on this platform. */
export const SEARCH_SHORTCUT_KEYS: readonly string[] = APPLE ? ["⌘", "K"] : ["Ctrl", "K"];
/** `aria-keyshortcuts` for the search shortcut on this platform. */
export const SEARCH_SHORTCUT_ARIA = APPLE ? "Meta+K" : "Control+K";

/** Opens search on ⌘K / Ctrl+K anywhere in the assistant. */
export function useAssistantSearchShortcut(open: () => void) {
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key.toLowerCase() !== "k" || !(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      event.preventDefault();
      open();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open]);
}
