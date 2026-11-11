import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useAssistantVoice } from "./use-assistant-voice";
import { ChatMessageList } from "@/components/assistant/chat-message";
import { VoiceCaptionText } from "@/components/assistant/voice-captions";
import { voiceDeliveryRevision } from "@/components/assistant/voice-caption-state";
import { voicePreferences } from "@/test/voice-fixture";
import type { ChatSessionState } from "@/lib/assistant/chat-types";
const f = vi.hoisted(() => ({ post: vi.fn(), create: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { post: f.post },
  apiClient: f.create,
}));
class Socket {
  static OPEN = 1;
  static current: Socket;
  readyState = 1;
  onmessage?: ({ data }: { data: string }) => void;
  send = vi.fn();
  close = vi.fn();
  constructor() {
    Socket.current = this;
  }
}
class Peer {
  static current: Peer;
  iceGatheringState = "complete";
  localDescription = { sdp: "v=0\r\nm=audio" };
  ontrack?: ({ streams }: { streams: MediaStream[] }) => void;
  constructor() {
    Peer.current = this;
  }
  addTrack() {}
  channel = {
    onmessage: null as null | ((event: { data: string }) => void),
    send() {},
  };
  createDataChannel() {
    return this.channel;
  }
  createOffer = async () => this.localDescription;
  setLocalDescription = async () => {};
  setRemoteDescription = async () => {};
  close() {}
}
const call = {
  id: "44444444-4444-4444-8444-444444444444",
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
  created_at: "2026-10-15T00:00:00Z",
  closed_at: null,
};
const caption = {
  id: "persisted",
  speaker: "assistant" as const,
  text: "Played. Pending.",
  sealed: true,
  complete: true,
  fragments: [
    { text: "Played. ", start_ms: 0, end_ms: 200 },
    { text: "Pending.", start_ms: 200, end_ms: 1000 },
  ],
};
const session: ChatSessionState = {
  clientId: "test",
  conversationId: "thread",
  title: "test",
  status: "completed_text",
  messages: [
    {
      id: "persisted",
      role: "assistant",
      content: caption.text,
      timestamp: 0,
      status: "complete",
    },
  ],
  expectedTurnCount: 0,
};
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function setup() {
  vi.useFakeTimers();
  const track = { enabled: true, stop: vi.fn() };
  const stream = {
    getTracks: () => [track],
    getAudioTracks: () => [track],
  } as unknown as MediaStream;
  vi.stubGlobal("navigator", {
    mediaDevices: { getUserMedia: vi.fn().mockResolvedValue(stream) },
  });
  vi.stubGlobal("RTCPeerConnection", Peer);
  vi.stubGlobal("WebSocket", Socket);
  vi.stubGlobal(
    "AudioContext",
    class {
      state = "running";
      resume = vi.fn().mockResolvedValue(undefined);
      close = vi.fn().mockResolvedValue(undefined);
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createAnalyser() {
        return {
          fftSize: 256,
          disconnect() {},
          getFloatTimeDomainData: (data: Float32Array) => data.fill(0.1),
        };
      }
    },
  );
  let paused = false;
  let audio!: HTMLAudioElement;
  const create = document.createElement.bind(document);
  vi.spyOn(document, "createElement").mockImplementation((name, options) => {
    const node = create(name, options);
    if (name === "audio") {
      audio = node as HTMLAudioElement;
      Object.defineProperty(audio, "srcObject", {
        value: null,
        writable: true,
      });
      Object.defineProperty(audio, "paused", { get: () => paused });
      audio.play = vi.fn().mockResolvedValue(undefined);
      audio.pause = vi.fn();
    }
    return node;
  });
  f.create.mockResolvedValue({ session: call, sdp_answer: "v=0\r\nm=audio" });
  function View() {
    const voice = useAssistantVoice("thread");
    return (
      <>
        <button onClick={() => void voice.start(voicePreferences)}>
          Start fixture
        </button>
        <button onClick={() => voice.muteSpeaker(!voice.speakerMuted)}>
          Toggle speaker
        </button>
        <output>
          {voice.connected ? "connected" : "ended"}:{voice.activity.playbackMs}:
          {voice.activity.output}
        </output>
        <ChatMessageList
          session={session}
          bottomInset={0}
          projectionVersion={voiceDeliveryRevision(
            voice.snapshot?.captions ?? [],
            voice.activity.playbackMs,
          )}
          renderMessage={() =>
            voice.snapshot ? (
              <VoiceCaptionText
                caption={voice.snapshot.captions[0]!}
                playbackMs={voice.activity.playbackMs}
              />
            ) : null
          }
        />
      </>
    );
  }
  const { container } = render(<View />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Start fixture" }));
  });
  act(() =>
    Peer.current.channel.onmessage?.({
      data: JSON.stringify({ type: "session.started" }),
    }),
  );
  return {
    container,
    audio,
    track,
    stream,
    pause: () => {
      paused = true;
    },
  };
}
it("real client/hook automatic termination preserves playback progress through the memoized persisted row", async () => {
  const { container, audio, track, stream } = await setup();
  act(() => {
    audio.currentTime = 0.25;
    Peer.current.ontrack?.({ streams: [stream] });
    Socket.current.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session: call,
        captions: [caption],
        tasks: [],
      }),
    });
    vi.advanceTimersByTime(80);
  });
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(1);
  expect(
    container.querySelectorAll('[data-voice-delivery="pending"]'),
  ).toHaveLength(1);
  act(() =>
    Socket.current.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session: { ...call, state: "closed" },
        captions: [caption],
        tasks: [],
      }),
    }),
  );
  expect(screen.getByRole("status")).toHaveTextContent("ended:250:0");
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(1);
  expect(
    container.querySelectorAll('[data-voice-delivery="pending"]'),
  ).toHaveLength(1);
  expect(track.stop).toHaveBeenCalledOnce();
});

it("caption timeline advances through speaker mute while output activity stays silent, and freezes on pause/end", async () => {
  const { container, audio, pause } = await setup();
  act(() => {
    Peer.current.ontrack?.({
      streams: [
        {
          getTracks: () => [],
          getAudioTracks: () => [],
        } as unknown as MediaStream,
      ],
    });
    Socket.current.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session: call,
        captions: [caption],
        tasks: [],
      }),
    });
    audio.currentTime = 0.1;
    vi.advanceTimersByTime(80);
  });
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Toggle speaker" }));
  expect(audio.muted).toBe(true);
  act(() => {
    audio.currentTime = 0.25;
    vi.advanceTimersByTime(80);
  });
  expect(screen.getByRole("status")).toHaveTextContent("connected:250:0");
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Toggle speaker" }));
  act(() => vi.advanceTimersByTime(80));
  expect(audio.muted).toBe(false);
  expect(screen.getByRole("status")).toHaveTextContent("connected:250:0.4");
  act(() => {
    pause();
    audio.currentTime = 1.2;
    vi.advanceTimersByTime(80);
  });
  expect(screen.getByRole("status")).toHaveTextContent("connected:250:0");
  act(() =>
    Socket.current.onmessage?.({
      data: JSON.stringify({
        type: "snapshot",
        session: { ...call, state: "closed" },
        captions: [caption],
        tasks: [],
      }),
    }),
  );
  expect(screen.getByRole("status")).toHaveTextContent("ended:250:0");
  expect(
    container.querySelectorAll('[data-voice-delivery="played"]'),
  ).toHaveLength(1);
  expect(
    container.querySelectorAll('[data-voice-delivery="pending"]'),
  ).toHaveLength(1);
});
