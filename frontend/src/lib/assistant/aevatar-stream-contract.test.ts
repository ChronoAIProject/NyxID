import { describe, expect, it } from "vitest";
import capturedStream from "@/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse?raw";
import { AGUIEventType } from "@/lib/assistant/agui-types";
import {
  createChatActorProjection,
  decodeActorFrame,
  reduceActorFrame,
} from "@/lib/assistant/chat-actor-state";
import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
} from "@/lib/assistant/runtime-event-semantics";
import {
  normalizeBackendSseFrame,
  SsePayloadDecoder,
} from "@/lib/assistant/sse-frame-normalizer";

describe("pinned Aevatar stream contract", () => {
  it("replays the raw capture through the canonical runtime and actor reducers", () => {
    const decoder = new SsePayloadDecoder();
    const payloads = [
      ...decoder.push(new TextEncoder().encode(capturedStream)),
      ...decoder.finish(),
    ];
    const runtime = createRuntimeEventAccumulator();
    let projection = createChatActorProjection();
    const actorFactTypes: string[] = [];

    expect(payloads).toHaveLength(6);
    for (const payload of payloads) {
      const raw: unknown = JSON.parse(payload);
      const event = normalizeBackendSseFrame(raw);

      expect(event).not.toBeNull();
      if (!event) continue;
      applyRuntimeEvent(runtime, event);
      if (event.type === AGUIEventType.RUN_STARTED) {
        projection = { ...projection, actorId: runtime.actorId };
      }

      const actorFrame = decodeActorFrame(raw);
      if (actorFrame.type !== "ignored") {
        actorFactTypes.push(actorFrame.type);
        projection = reduceActorFrame(projection, actorFrame);
      }
    }

    expect(runtime).toMatchObject({
      actorId: "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
      assistantText: "Blue is a color.  \nGreen is a color.",
      errorText: "",
      runId: "turn-server-owned-1",
    });
    expect(runtime.events.at(-1)?.type).toBe(AGUIEventType.RUN_FINISHED);
    expect(projection).toMatchObject({
      actorId: "nyxid-chat-f8369965a444433f92ec50e67ad8ee52",
      progressSequence: 0,
    });
    expect(actorFactTypes).toEqual([]);
    expect(projection.actions.size).toBe(0);
    expect(projection.conflicts).toHaveLength(0);
  });
});
