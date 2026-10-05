import { useCallback, useEffect, useRef, useState } from "react";
import { AssistantVoiceClient } from "@/lib/assistant-voice-client";
import { api } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import type {
  VoicePreferences,
  VoiceSnapshot,
  VoiceSession,
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
  const [conflictSession, setConflictSession] =
    useState<VoiceSession | null>(null);
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
      setConflictSession(null);
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
        (session) => {
          if (generation.current === ticket) setConflictSession(session);
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
    else if (conflictSession) {
      try {
        await api.post(
          `/assistant/nyxagent/conversations/${thread}/voice-sessions/${conflictSession.id}/control`,
          {
            command_id: crypto.randomUUID(),
            expected_revision: conflictSession.control_revision,
            action: "end",
          },
        );
      } catch {
        setError("The current call could not be ended. Please try again.");
      }
    }
    setConflictSession(null);
  }, [conflictSession, thread]);
  const rejoin = useCallback(() => {
    if (!conflictSession?.resumable || client.current) return;
    const ticket = ++generation.current;
    const next = new AssistantVoiceClient(
      thread,
      (value) => {
        if (generation.current === ticket) {
          setConflictSession(null);
          setSnapshot(value);
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
          setConnected(false);
        }
      },
      (session) => {
        if (generation.current === ticket) setConflictSession(session);
      },
    );
    client.current = next;
    setError(null);
    setSnapshot(null);
    next.rejoin(conflictSession);
  }, [conflictSession, thread]);
  return {
    holding,
    connected,
    snapshot,
    starting,
    error,
    conflictSession,
    rejoin,
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
