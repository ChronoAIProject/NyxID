/// <reference lib="webworker" />
import createRive, {
  type Artboard,
  type StateMachineInstance,
  type ViewModelInstance,
  type Node,
  type RiveCanvas,
  type File,
} from "@rive-app/canvas-advanced";
import wasmUrl from "@rive-app/canvas-advanced/rive.wasm?url";
import rigUrl from "../../../../../shared/agent-avatars/rive/nyx-emotes.riv?url";
import { type EmotePose } from "../../../../../shared/agent-avatars/emotes";
import { PROP_NODES } from "../../../../../shared/agent-avatars/props";

interface Entry {
  artboard: Artboard;
  machine: StateMachineInstance;
  values: ViewModelInstance;
  eye: Node;
  props: Record<string, Node>;
  active: boolean;
  settled: boolean;
  last: string;
}
let runtime: RiveCanvas;
let file: File;
let loading: Promise<void> | undefined;
let timer: ReturnType<typeof setInterval> | undefined;
let at = 0;
const entries = new Map<number, Entry>();
interface Pending {
  emote: number;
  active: boolean;
  offset: number;
}
// Home cards are static; this also bounds unusual consumers of the renderer.
const MAX_SCENES = 24;
const pending = new Map<number, Pending>();
async function load() {
  if (!loading) {
    const attempt = (async () => {
      runtime = await createRive({ locateFile: () => wasmUrl });
      const response = await fetch(rigUrl);
      if (!response.ok) throw Error("Rive emote asset unavailable");
      file = await runtime.load(new Uint8Array(await response.arrayBuffer()));
      if (!file) throw Error("Rive emote asset could not load");
    })();
    loading = attempt;
    void attempt.catch(() => {
      if (loading === attempt) loading = undefined;
    });
  }
  return loading;
}
function tick() {
  const now = performance.now();
  const dt = Math.min(0.05, Math.max(0, (now - at) / 1000));
  at = now;
  const poses: { id: number; pose: EmotePose }[] = [];
  for (const [id, entry] of entries) {
    if (!entry.active || entry.settled) continue;
    entry.settled = !entry.machine.advance(dt);
    entry.artboard.advance(dt);
    const pose: EmotePose = {
      x: entry.eye.x,
      y: entry.eye.y,
      openness: entry.eye.scaleY,
      props: Object.fromEntries(
        Object.entries(entry.props).map(([name, node]) => [
          name,
          {
            x: node.x,
            y: node.y,
            rotation: node.rotation,
            scaleX: node.scaleX,
            scaleY: node.scaleY,
          },
        ]),
      ),
    };
    const signature = JSON.stringify(pose, (_key, value) =>
      typeof value === "number" ? +value.toFixed(4) : value,
    );
    if (signature !== entry.last) {
      entry.last = signature;
      poses.push({ id, pose });
    }
  }
  if (poses.length) self.postMessage({ type: "poses", poses });
  schedule();
}
function schedule() {
  const active = [...entries.values()].some(
    (entry) => entry.active && !entry.settled,
  );
  if (active && !timer) {
    at = performance.now();
    timer = setInterval(tick, 1000 / 60);
  }
  if (!active && timer) {
    clearInterval(timer);
    timer = undefined;
  }
}
self.onmessage = async (
  event: MessageEvent<{
    type: string;
    id: number;
    emote?: number;
    active?: boolean;
    offset?: number;
  }>,
) => {
  const message = event.data;
  if (message.type === "remove") {
    const entry = entries.get(message.id);
    pending.delete(message.id);
    if (entry) {
      entry.machine.delete();
      entry.artboard.delete();
      entry.values.delete();
      entries.delete(message.id);
    }
    schedule();
    return;
  }
  if (message.type === "add") {
    if (entries.size + pending.size >= MAX_SCENES) {
      self.postMessage({ type: "failed", id: message.id });
      return;
    }
    const queued: Pending = {
      emote: message.emote ?? 0,
      active: message.active ?? true,
      offset: message.offset ?? 0,
    };
    pending.set(message.id, queued);
    try {
      await load();
      if (pending.get(message.id) !== queued) return;
      const artboard = file.artboardByName("NyxEmotes");
      const values = file.viewModelByName("AvatarEmote").instance();
      const machine = new runtime.StateMachineInstance(
        artboard.stateMachineByName("Emotes"),
        artboard,
      );
      machine.bindViewModelInstance(values);
      values.number("emote").value = queued.emote;
      machine.advance(0);
      artboard.advance(0);
      // Desynchronise long idle holds without replaying a greeting on mount.
      for (let t = 0; t < queued.offset; t += 1 / 60) {
        machine.advance(1 / 60);
        artboard.advance(1 / 60);
      }
      entries.set(message.id, {
        artboard,
        values,
        machine,
        eye: artboard.node("EyePose"),
        props: Object.fromEntries(
          PROP_NODES.map(({ name }) => [name, artboard.node(name)]),
        ),
        active: queued.active,
        settled: false,
        last: "",
      });
      self.postMessage({ type: "ready", id: message.id });
      schedule();
      tick();
    } catch (error) {
      self.postMessage({
        type: "failed",
        id: message.id,
        reason: error instanceof Error ? error.message : String(error),
      });
    } finally {
      pending.delete(message.id);
    }
    return;
  }
  const queued = pending.get(message.id);
  if (queued) {
    if (message.type === "state") {
      queued.emote = message.emote ?? 0;
      queued.offset = 0;
    }
    if (message.type === "active") queued.active = Boolean(message.active);
    return;
  }
  const entry = entries.get(message.id);
  if (!entry) return;
  if (message.type === "state") {
    entry.values.number("emote").value = message.emote ?? 0;
    entry.settled = false;
  }
  if (message.type === "active") entry.active = Boolean(message.active);
  schedule();
};
