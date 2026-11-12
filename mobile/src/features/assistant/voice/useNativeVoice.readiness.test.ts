import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../assistantHookTestHarness";
import {
  AssistantVoiceClient,
  type AssistantVoiceClientOptions,
  type VoiceMediaEvent,
} from "./AssistantVoiceClient";

for (const order of [
  "control-first",
  "provider-first",
  "provider-timeout",
] as const) {
  test(`real hook and client readiness: ${order}`, async () => {
    let time = 1000;
    let expire = () => {};
    const session = {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      created_at: "2026-11-06T00:00:00Z",
      closed_at: null,
      generation: 1,
      state: "active",
      muted: false,
      input_muted: false,
      end_requested: false,
      control_revision: 0,
      observed_seconds: 0,
      reserved_until: 15,
      final_usage_confirmed: false,
      end_reason: null,
      idle_warning: false,
      resumable: true,
    };
    const preferences = {
      service_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
      connection_id: null,
      key_source: "platform" as const,
      model: "live",
      voice: "alloy",
      input_mode: "automatic" as const,
      language: null,
      notify_on_completion: false,
    };
    const socket = {
      readyState: 1,
      binaryType: "",
      bufferedAmount: 0,
      onmessage: null as any,
      onclose: null as any,
      onerror: null as any,
      send: () => {},
      close: () => {},
    };
    let media!: Media;
    class Media {
      input = false;
      listener?: (event: VoiceMediaEvent) => void;
      constructor() {
        media = this;
      }
      subscribe(cb: (event: VoiceMediaEvent) => void) {
        this.listener = cb;
        return () => {
          this.listener = undefined;
        };
      }
      async prepare() {
        return "offer";
      }
      async answer() {}
      setInput(value: boolean) {
        this.input = value;
      }
      close() {
        this.input = false;
      }
      flush() {}
      hold() {}
      muteSpeaker() {}
      async resume() {}
      enqueue() {}
      playback() {
        return { playback_ms: 0, playback_audible: false };
      }
    }
    class Client extends AssistantVoiceClient {
      constructor(options: AssistantVoiceClientOptions) {
        super({
          ...options,
          clock: {
            setTimeout: (cb, ms) => {
              // 15 s readiness; unmount also arms End's 7 s drain deadline.
              assert.ok(ms === 15000 || ms === 7000, `unexpected timer ${ms}`);
              if (ms === 15000) expire = cb;
              return 1;
            },
            clearTimeout: () => {
              expire = () => {};
            },
            setInterval: () => 1,
            clearInterval: () => {},
          },
        });
      }
    }
    const environment = {
      epoch: 1,
      api: {
        options: async () => ({
          options: [
            {
              ...preferences,
              available: true,
              voice: { protocol: "openai_live" },
            },
          ],
        }),
        savePreferences: async () => {},
        start: async () => ({ session, sdp_answer: "answer" }),
        control: async () => session,
      },
      createSocket: async () => socket,
    };
    const h = hookHarness<any>(
      {
        "react-native": {
          Platform: { OS: "ios" },
          AppState: {
            currentState: "active",
            addEventListener: () => ({ remove: () => {} }),
          },
        },
        "expo-camera": {
          Camera: {
            getMicrophonePermissionsAsync: async () => ({ granted: true }),
            requestMicrophonePermissionsAsync: async () => ({ granted: true }),
          },
        },
        "expo-crypto": { randomUUID: () => session.id },
        "../../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
        "./AssistantVoiceClient": { AssistantVoiceClient: Client },
        "./VoiceMediaBridge": { NativeVoiceMedia: Media },
      },
      {
        Date: class extends Date {
          static override now() {
            return time;
          }
        },
      },
    );
    const { useNativeVoice } = h.load(
      fileURLToPath(new URL("./useNativeVoice.ts", import.meta.url)),
    ) as any;
    h.mount(() =>
      useNativeVoice(
        "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        environment,
        () => {},
      ),
    );
    await h.value().start(preferences, "openai_live");
    const control = () =>
      socket.onmessage({
        data: JSON.stringify({
          type: "snapshot",
          session,
          captions: [],
          tasks: [],
        }),
      });
    const provider = () => media.listener?.({ type: "provider-ready" });
    (order === "provider-first" ? provider : control)();
    assert.equal(h.flush().connected, false);
    assert.equal(h.value().starting, true);
    assert.equal(h.value().connectedAt, undefined);
    assert.equal(media.input, false);
    time = 5000;
    if (order === "provider-timeout") {
      expire();
      assert.match(h.flush().error, /timed out/);
      assert.equal(h.value().connectedAt, undefined);
      assert.equal(h.value().connected, false);
      provider();
      assert.equal(h.flush().connectedAt, undefined);
    } else {
      (order === "provider-first" ? control : provider)();
      assert.equal(h.flush().connectedAt, 5000);
      assert.equal(h.value().connected, true);
      assert.equal(h.value().starting, false);
      assert.equal(media.input, true);
      time = 9000;
      control();
      provider();
      assert.equal(h.flush().connectedAt, 5000);
      expire();
      assert.equal(h.flush().error, undefined);
    }
    h.unmount();
  });
}
