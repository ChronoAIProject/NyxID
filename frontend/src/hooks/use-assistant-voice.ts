import { useCallback, useEffect, useRef, useState } from "react";
import { AssistantVoiceClient } from "@/lib/assistant-voice-client";
import { useAuthStore } from "@/stores/auth-store";
import type {
  VoicePreferences,
  VoiceSnapshot,
} from "@/schemas/assistant-voice";

export function useAssistantVoice(thread: string) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useRef<AssistantVoiceClient | null>(null);
  const generation = useRef(0);
  const [holding, setHolding] = useState(false);
  const [connected, setConnected] = useState(false);
  const [snapshot, setSnapshot] = useState<VoiceSnapshot | null>(null);
  const [speakerMuted, setSpeakerMuted] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(
    () => () => {
      generation.current++;
      const previous = client.current;
      client.current = null;
      if (previous) void previous.end();
    },
    [actor, thread],
  );
  const start = useCallback(
    async (
      preferences: VoicePreferences,
      protocol?: "openai_live" | "xai_realtime",
    ) => {
      if (client.current) return;
      const ticket = ++generation.current;
      setStarting(true);
      setError(null);
      setSnapshot(null);
      setSpeakerMuted(false);
      const next = new AssistantVoiceClient(
        thread,
        (value) => {
          if (generation.current === ticket) {
            setSnapshot(value);
            if (protocol === "xai_realtime")
              setStarting(value.session.state === "starting");
            setConnected(
              value.session.state === "active" ||
                value.session.state === "closing",
            );
          }
        },
        (message) => {
          if (generation.current === ticket) setError(message);
        },
        () => {
          if (generation.current === ticket) {
            client.current = null;
            setStarting(false);
            setConnected(false);
          }
        },
      );
      client.current = next;
      await next.start(preferences, protocol);
      if (generation.current === ticket) {
        if (protocol !== "xai_realtime" || next.isClosed) setStarting(false);
        if (next.isClosed) client.current = null;
      }
    },
    [thread],
  );
  const end = useCallback(async () => {
    const previous = client.current;
    client.current = null;
    generation.current++;
    setHolding(false);
    setStarting(false);
    setConnected(false);
    setSnapshot(null);
    if (previous) await previous.end();
  }, []);
  return {
    holding,
    connected,
    snapshot,
    starting,
    error,
    start,
    end,
    speakerMuted,
    muteSpeaker: (muted: boolean) => {
      client.current?.muteSpeaker(muted);
      setSpeakerMuted(muted);
    },
    hold: (pressed: boolean) => {
      setHolding(pressed);
      client.current?.hold(pressed);
    },
    mute: (muted: boolean) => client.current?.mute(muted),
    resumeAudio: () => client.current?.resumeAudio(),
    stopTask: (id: string) => client.current?.stopTask(id),
  };
}
