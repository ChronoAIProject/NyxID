import { useSyncExternalStore } from "react";

let pollingDeadline = 0;
const subscribers = new Set<() => void>();
let tickInterval: ReturnType<typeof setInterval> | null = null;

function notifySubscribers() {
  for (const cb of subscribers) cb();
}

function startTick() {
  if (tickInterval) return;
  tickInterval = setInterval(() => {
    if (Date.now() >= pollingDeadline) {
      if (tickInterval) {
        clearInterval(tickInterval);
        tickInterval = null;
      }
      notifySubscribers();
    }
  }, 1000);
}

function stopTickIfIdle() {
  if (subscribers.size === 0 && tickInterval) {
    clearInterval(tickInterval);
    tickInterval = null;
  }
}

export function startPushPolling(durationMs = 15_000) {
  pollingDeadline = Date.now() + durationMs;
  notifySubscribers();
  startTick();
}

function subscribe(callback: () => void) {
  subscribers.add(callback);
  if (Date.now() < pollingDeadline) startTick();
  return () => {
    subscribers.delete(callback);
    stopTickIfIdle();
  };
}

function getSnapshot(): boolean {
  return Date.now() < pollingDeadline;
}

export function usePushPollingActive(): boolean {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
