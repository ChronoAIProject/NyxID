import { useEffect, useRef } from "react";

type ScrollCallback = (scrollY: number, vh: number) => void;

// Single global scroll listener shared by all subscribers
let listeners: Set<ScrollCallback> | null = null;
let rafId = 0;

function handleScroll() {
  cancelAnimationFrame(rafId);
  rafId = requestAnimationFrame(() => {
    if (!listeners || listeners.size === 0) return;
    const scrollY = window.scrollY;
    const vh = window.innerHeight;
    listeners.forEach((cb) => cb(scrollY, vh));
  });
}

function subscribe(cb: ScrollCallback) {
  if (!listeners) {
    listeners = new Set();
    window.addEventListener("scroll", handleScroll, { passive: true });
  }
  listeners.add(cb);
}

function unsubscribe(cb: ScrollCallback) {
  if (!listeners) return;
  listeners.delete(cb);
  if (listeners.size === 0) {
    window.removeEventListener("scroll", handleScroll);
    cancelAnimationFrame(rafId);
    listeners = null;
  }
}

export function useScroll(callback: ScrollCallback) {
  const cbRef = useRef<ScrollCallback>(callback);

  // Keep ref in sync without re-subscribing
  useEffect(() => {
    cbRef.current = callback;
  });

  useEffect(() => {
    // Stable wrapper that always calls the latest callback
    const stable: ScrollCallback = (sy, vh) => cbRef.current(sy, vh);

    subscribe(stable);

    // Fire once on mount
    stable(window.scrollY, window.innerHeight);

    return () => {
      unsubscribe(stable);
    };
  }, []); // empty deps — mount/unmount only
}
