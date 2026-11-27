import type { EmotePose } from "../../../../../shared/agent-avatars/emotes";

type Listener = {
  pose: (pose: EmotePose) => void;
  ready: (ready: boolean) => void;
};
let worker: Worker | undefined;
let nextId = 0;
const listeners = new Map<number, Listener>();
let release: ReturnType<typeof setTimeout> | undefined;

/** One lazy worker/runtime for every visible emote. WASM stays off the UI thread. */
export function animateRiveEyes(listener: Listener, emote: number, offset = 0) {
  if (typeof Worker === "undefined") {
    listener.ready(false);
    return null;
  }
  clearTimeout(release);
  if (!worker) {
    try {
      worker = new Worker(new URL("./rive-motion.worker.ts", import.meta.url), {
        type: "module",
      });
    } catch {
      listener.ready(false);
      return null;
    }
    worker.onmessage = (
      event: MessageEvent<{
        type: string;
        id?: number;
        poses?: { id: number; pose: EmotePose }[];
      }>,
    ) => {
      if (event.data.type === "poses")
        for (const { id, pose } of event.data.poses ?? [])
          listeners.get(id)?.pose(pose);
      else if (event.data.id !== undefined)
        listeners.get(event.data.id)?.ready(event.data.type === "ready");
    };
    const instance = worker;
    worker.onerror = () => {
      if (worker !== instance) return;
      for (const entry of listeners.values()) entry.ready(false);
      listeners.clear();
      worker?.terminate();
      worker = undefined;
    };
  }
  const id = ++nextId;
  const instance = worker;
  let stopped = false;
  listeners.set(id, listener);
  instance.postMessage({ type: "add", id, emote, offset });
  return {
    state: (value: number) =>
      instance.postMessage({ type: "state", id, emote: value }),
    active: (active: boolean) =>
      instance.postMessage({ type: "active", id, active }),
    stop: () => {
      if (stopped) return;
      stopped = true;
      listeners.delete(id);
      instance.postMessage({ type: "remove", id });
      if (!listeners.size && worker === instance)
        release = setTimeout(() => {
          worker?.terminate();
          worker = undefined;
        }, 1500);
    },
  };
}
