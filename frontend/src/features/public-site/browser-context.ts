import { useSyncExternalStore } from "react";
const EMPTY_CONTEXT = "\0";
function subscribe(listener: () => void) {
  window.addEventListener("popstate", listener);
  window.addEventListener("hashchange", listener);
  window.addEventListener("nyxid:public-url-context", listener);
  return () => {
    window.removeEventListener("popstate", listener);
    window.removeEventListener("hashchange", listener);
    window.removeEventListener("nyxid:public-url-context", listener);
  };
}
/** Hydration reads the canonical snapshot first, then the actual browser URL. */
export function useBrowserContext() {
  const value = useSyncExternalStore(
    subscribe,
    () => `${window.location.search}\0${window.location.hash}`,
    () => EMPTY_CONTEXT,
  );
  const [search = "", hash = ""] = value.split("\0");
  return { search, hash };
}
