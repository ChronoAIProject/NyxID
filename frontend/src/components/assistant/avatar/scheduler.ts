/** One timer and one rich owner. A failed renderer releases its slot. */
interface Entry {
  rich: boolean;
  priority: number;
  tick: (seconds: number) => void;
  onFailure?: () => void;
  start: number;
}
const entries = new Map<symbol, Entry>();
let frame = 0,
  last = 0,
  failures = 0,
  registrations = 0;
function loop(now: number) {
  frame = 0;
  try {
    if (now - last >= 1000 / 12 || now < last) {
      last = now;
      const richOwner = [...entries.values()]
        .filter((e) => e.rich)
        .sort((a, b) => b.priority - a.priority)[0];
      for (const [id, entry] of entries) {
        if (entry.rich && entry !== richOwner) continue;
        try {
          entry.tick(Math.max(0, (now - entry.start) / 1000));
        } catch {
          entries.delete(id);
          failures++;
          try {
            entry.onFailure?.();
          } catch {
            /* Failure handling must not stop other renderers. */
          }
        }
      }
    }
  } finally {
    if (entries.size && !frame) frame = requestAnimationFrame(loop);
  }
}
export function animateAvatar(
  rich: boolean,
  tick: Entry["tick"],
  priority = 0,
  onFailure?: () => void,
) {
  const id = Symbol();
  registrations++;
  entries.set(id, {
    rich,
    priority,
    tick,
    onFailure,
    start: performance.now(),
  });
  if (!frame) {
    last = performance.now() - 1000 / 12;
    frame = requestAnimationFrame(loop);
  }
  return () => {
    entries.delete(id);
    if (!entries.size && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
  };
}
export function avatarSchedulerInfo() {
  return {
    active: entries.size,
    rich: Math.min(1, [...entries.values()].filter((e) => e.rich).length),
    fps: 12,
    failures,
    registrations,
  };
}
