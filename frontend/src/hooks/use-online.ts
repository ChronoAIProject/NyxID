import { useSyncExternalStore } from "react";

function subscribe(onChange: () => void) {
  window.addEventListener("online", onChange);
  window.addEventListener("offline", onChange);
  return () => {
    window.removeEventListener("online", onChange);
    window.removeEventListener("offline", onChange);
  };
}

/**
 * Dev-server preview of the offline banner: `?offline=1` reads as offline.
 * Compiled out of production builds.
 */
function previewOffline(): boolean {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get("offline") === "1";
}

/** Whether the browser has a network connection. */
export function useOnline(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => navigator.onLine && !previewOffline(),
    () => true,
  );
}
