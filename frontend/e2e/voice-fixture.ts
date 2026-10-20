import type { Page } from "@playwright/test";
import { voiceOption } from "../src/test/voice-fixture";

/** Explicit local media + NyxID wire fixture. Never connects to a provider. */
export async function installVoiceFixture(
  page: Page,
  permission: "granted" | "pending" | "denied" = "granted",
) {
  const calls = { starts: 0, ends: 0, dictations: 0, stops: 0, pcm: 0 };
  await page.addInitScript(
    ({ permission }) => {
      const f = {
        requests: 0,
        stopped: 0,
        captureDevices: [] as (string | null)[],
        grant: () => {},
        deny: () => {},
        revoked: () => {},
      };
      Object.assign(window, { __composerVoiceFixture: f });
      function stream() {
        const c = new AudioContext();
        void c.resume();
        const oscillator = c.createOscillator();
        oscillator.frequency.value = 180;
        const gain = c.createGain();
        gain.gain.value = 0.08;
        const destination = c.createMediaStreamDestination();
        oscillator.connect(gain).connect(destination);
        oscillator.start();
        const audio = destination.stream;
        audio.getTracks().forEach((track) => {
          const stop = track.stop.bind(track);
          track.stop = () => {
            f.stopped++;
            stop();
            oscillator.stop();
            void c.close().catch(() => {});
          };
        });
        f.revoked = () =>
          audio
            .getTracks()
            .forEach((track) => track.dispatchEvent(new Event("ended")));
        return audio;
      }
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: {
          enumerateDevices: async () => [
            {
              kind: "audioinput",
              deviceId: "built-in",
              groupId: "input",
              label: "MacBook Air Microphone",
            },
          ],
          addEventListener() {},
          removeEventListener() {},
          getUserMedia: (constraints: MediaStreamConstraints) => {
            f.requests++;
            const device =
              typeof constraints.audio === "object"
                ? constraints.audio.deviceId
                : undefined;
            const exact =
              typeof device === "object" && !Array.isArray(device)
                ? device.exact
                : device;
            f.captureDevices.push(typeof exact === "string" ? exact : null);
            if (permission === "denied")
              return Promise.reject(
                new DOMException(
                  "Fixture permission denied",
                  "NotAllowedError",
                ),
              );
            if (permission === "pending")
              return new Promise<MediaStream>((resolve, reject) => {
                f.grant = () => resolve(stream());
                f.deny = () =>
                  reject(new DOMException("Denied", "NotAllowedError"));
              });
            return Promise.resolve(stream());
          },
        },
      });
      class Peer {
        connectionState = "connected";
        iceGatheringState = "complete";
        localDescription = { sdp: "v=0\r\nm=audio" };
        ontrack?: (event: {
          streams: MediaStream[];
          track: MediaStreamTrack;
        }) => void;
        channel = {
          onmessage: null as null | ((event: { data: string }) => void),
          send() {},
          close() {},
        };
        addTrack() {}
        addEventListener() {}
        removeEventListener() {}
        close() {}
        createDataChannel() {
          return this.channel;
        }
        async createOffer() {
          return this.localDescription;
        }
        async setLocalDescription() {}
        async setRemoteDescription() {
          this.channel.onmessage?.({
            data: JSON.stringify({ type: "session.started" }),
          });
          const audio = stream();
          this.ontrack?.({ streams: [audio], track: audio.getTracks()[0]! });
        }
      }
      Object.assign(window, { RTCPeerConnection: Peer });
    },
    { permission },
  );
  const session = {
    id: "33333333-3333-4333-8333-333333333333",
    generation: 1,
    state: "active",
    muted: false,
    input_muted: false,
    end_requested: false,
    control_revision: 0,
    observed_seconds: 0,
    reserved_until: 30,
    final_usage_confirmed: false,
    end_reason: null,
    idle_warning: false,
    created_at: new Date().toISOString(),
    closed_at: null,
    resumable: true,
  };
  const priced = {
    ...voiceOption,
    pricing: {
      ...voiceOption.pricing!,
      components: [
        { metric: "requests", credits_per_unit: "0.02", sync_status: "synced" },
      ],
    },
  };
  await page.route(
    "**/api/v1/assistant/nyxagent/dictation/options?*",
    (route) =>
      route.fulfill({
        json: {
          options: [priced],
          transcription_model: "gpt-live-transcribe",
          max_seconds: 120,
        },
      }),
  );
  await page.route("**/api/v1/assistant/nyxagent/voice/options?*", (route) =>
    route.fulfill({
      json: { options: [{ ...priced, model_label: "Fixture Live" }] },
    }),
  );
  let settings = {
    access_mode: "nyxmode",
    skip_destructive_confirmation: false,
    max_live_subagents: 4,
    max_concurrent_subagent_turns: 2,
    max_live_subagents_limit: 8,
    max_concurrent_subagent_turns_limit: 4,
  };
  await page.route("**/api/v1/assistant/nyxagent/settings", (route) => {
    if (route.request().method() === "PUT")
      settings = { ...settings, ...route.request().postDataJSON() };
    return route.fulfill({ json: settings });
  });
  await page.route("**/voice-sessions/active", (route) =>
    route.fulfill({ json: { session: null } }),
  );
  await page.route("**/voice-sessions", (route) => {
    calls.starts++;
    return route.fulfill({ json: { session, sdp_answer: "v=0\r\nm=audio" } });
  });
  await page.route("**/voice-sessions/*/control", (route) => {
    const body = route.request().postDataJSON() as { action: string };
    if (body.action === "end") calls.ends++;
    return route.fulfill({
      json: {
        ...session,
        state: body.action === "end" ? "closed" : "active",
        muted: body.action === "mute",
        control_revision: 1,
      },
    });
  });
  await page.routeWebSocket("**/voice-sessions/*/stream", (socket) => {
    socket.send(
      JSON.stringify({
        type: "snapshot",
        session,
        captions: [
          {
            id: "fixture-user",
            speaker: "user",
            text: "A real event-shaped live caption",
            sealed: true,
            complete: true,
            start_ms: 0,
            end_ms: 500,
            fragments: [],
          },
          {
            id: "fixture-assistant",
            speaker: "assistant",
            text: "Played. Pending.",
            sealed: true,
            complete: true,
            start_ms: 0,
            end_ms: 60000,
            fragments: [
              { text: "Played. ", start_ms: 0, end_ms: 200 },
              { text: "Pending.", start_ms: 200, end_ms: 60000 },
            ],
          },
        ],
        tasks: [],
      }),
    );
    socket.onMessage(() => {});
  });
  await page.routeWebSocket("**/dictation", (socket) => {
    socket.onMessage((message) => {
      if (typeof message !== "string") {
        calls.pcm++;
        return;
      }
      const request = JSON.parse(message) as { type?: string };
      if (request.type === "stop") {
        calls.stops++;
        socket.send(
          JSON.stringify({
            type: "completed",
            item_id: "B",
            order: 1,
            text: "second",
          }),
        );
        socket.send(JSON.stringify({ type: "stopped", confirmed: true }));
        return;
      }
      calls.dictations++;
      socket.send(
        JSON.stringify({
          type: "ready",
          session_id: session.id,
          model: "gpt-live-transcribe",
        }),
      );
      socket.send(
        JSON.stringify({
          type: "delta",
          item_id: "B",
          order: 1,
          text: "second interim",
        }),
      );
      socket.send(
        JSON.stringify({
          type: "completed",
          item_id: "A",
          order: 0,
          text: "first",
        }),
      );
    });
  });
  return calls;
}
