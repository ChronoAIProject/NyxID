import { base64, type DesktopFrame } from './protocol';

export type CanvasFrame = {
  generation: number;
  sequence: string;
  jpeg: string;
  width: number;
  height: number;
  rectangle?: { width: number; height: number; x: number; y: number; base: string };
};

/** One bridge/decode in flight, three compressed frames waiting. Only the
 * dispatched JPEG is base64 encoded; acknowledgements never contain pixels. */
let generationClock = 0;
export function createCompositor(send: (frame: CanvasFrame) => void, refresh: () => void) {
  let generation = ++generationClock;
  let last = 0n;
  let seen = 0n;
  let inFlight: DesktopFrame | undefined;
  let waiting: DesktopFrame[] = [];
  const pump = () => {
    if (inFlight) return;
    while (waiting.length) {
      const frame = waiting.shift()!;
      if (frame.sequence <= last) continue;
      if (frame.rectangle?.base && frame.rectangle.base !== last) { refresh(); continue; }
      inFlight = frame;
      send({ generation, sequence: frame.sequence.toString(), jpeg: base64(frame.jpeg),
        width: frame.width, height: frame.height,
        rectangle: frame.rectangle && { ...frame.rectangle, base: frame.rectangle.base.toString() } });
      return;
    }
  };
  return {
    reset() { generation = ++generationClock; last = 0n; seen = 0n; inFlight = undefined; waiting = []; return generation; },
    enqueue(frame: DesktopFrame) {
      if (frame.sequence <= seen) return;
      seen = frame.sequence;
      if (!frame.rectangle || frame.rectangle.base === 0n) waiting = [];
      if (waiting.length + Number(!!inFlight) >= 4) { refresh(); return; }
      waiting.push(frame); pump();
    },
    acknowledge(result: { generation: number; sequence: string; ok: boolean }) {
      if (result.generation !== generation || result.sequence !== inFlight?.sequence.toString()) return false;
      if (result.ok) last = inFlight.sequence;
      else { last = 0n; waiting = []; refresh(); }
      inFlight = undefined; pump(); return result.ok;
    },
    depth: () => waiting.length + Number(!!inFlight),
  };
}
