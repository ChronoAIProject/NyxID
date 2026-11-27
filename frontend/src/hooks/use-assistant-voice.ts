import { useCallback, useEffect, useRef, useState } from "react";
import {
  microphoneDevice,
  silentActivity,
  type PreparedMicrophone,
} from "@/lib/microphone-capture";
import {
  AssistantVoiceClient,
  type VoiceConnectionStatus,
} from "@/lib/assistant-voice-client";
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
  const [activity, setActivity] = useState(silentActivity);
  const [holding, setHolding] = useState(false);
  const [inputAccepting, setInputAccepting] = useState(false);
  const [connected, setConnected] = useState(false);
  const [snapshot, setSnapshot] = useState<VoiceSnapshot | null>(null);
  const [speakerMuted, setSpeakerMuted] = useState(false);
  const [starting, setStarting] = useState(false);
  const transport = useRef<{
    sessionId?: string;
    last?: VoiceConnectionStatus;
    sequence: number;
  }>({ sequence: 0 });
  const [restoredEvent, setRestoredEvent] = useState<{
    id: string;
    sessionId: string;
  } | null>(null);
  function observeStatus(status: VoiceConnectionStatus) {
    const previous = transport.current;
    if (
      previous.last === "reconnecting" &&
      status === "ready" &&
      previous.sessionId
    ) {
      previous.sequence++;
      setRestoredEvent({
        id: `${previous.sessionId}:${previous.sequence}`,
        sessionId: previous.sessionId,
      });
    }
    previous.last = status;
  }
  function observeSession(sessionId: string) {
    if (transport.current.sessionId !== sessionId) {
      transport.current = { sessionId, sequence: 0 };
      setRestoredEvent(null);
    }
  }
  useEffect(() => {
    if (!restoredEvent) return;
    const timer = setTimeout(() => setRestoredEvent(null), 1200);
    return () => clearTimeout(timer);
  }, [restoredEvent]);
  const [connectionStatus, setConnectionStatus] =
    useState<VoiceConnectionStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [conflictSession, setConflictSession] = useState<VoiceSession | null>(
    null,
  );
  useEffect(() => {
    // Actor/thread changes are an ownership boundary for this external media client.
    setInputAccepting(false);
    setHolding(false);
    setStarting(false);
    setConnected(false);
    setConnectionStatus(null);
    transport.current = { sequence: 0 };
    setRestoredEvent(null);
    return () => {
      // This counter fences live media callbacks; cleanup must advance its current value.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      generation.current++;
      const previous = client.current;
      client.current = null;
      if (previous) void previous.end();
    };
  }, [actor, thread]);
  const start = useCallback(
    async (
      preferences: VoicePreferences,
      protocol?: "openai_live" | "xai_realtime",
      beforeProvider?: () => Promise<void>,
      preparation?: PreparedMicrophone,
    ) => {
      if (client.current) return;
      const ticket = ++generation.current;
      setInputAccepting(false);
      transport.current = { sequence: 0 };
      setRestoredEvent(null);
      setStarting(true);
      setError(null);
      setSnapshot(null);
      setConflictSession(null);
      setSpeakerMuted(false);
      const next = new AssistantVoiceClient(
        thread,
        (value) => {
          if (generation.current === ticket) {
            observeSession(value.session.id);
            setSnapshot(value);
            if (value.session.state === "closing") {
              setStarting(false);
              setConnected(true);
              setHolding(false);
            }
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
            setInputAccepting(false);
            setHolding(false);
            setConnectionStatus(null);
          }
        },
        (session) => {
          if (generation.current === ticket) setConflictSession(session);
        },
        (value) => {
          if (generation.current === ticket) setActivity(value);
        },
        (status) => {
          if (generation.current === ticket) {
            observeStatus(status);
            setConnectionStatus(status);
            setStarting(status !== "ready");
            setConnected(status === "ready");
            if (status !== "ready") {
              setHolding(false);
              setInputAccepting(false);
            }
          }
        },
        (enabled) => {
          if (generation.current === ticket) setInputAccepting(enabled);
        },
      );
      client.current = next;
      await next.start(
        preferences,
        protocol,
        beforeProvider,
        preparation,
        microphoneDevice(actor ?? ""),
      );
      if (generation.current === ticket && next.isClosed) {
        setInputAccepting(false);
        setStarting(false);
        client.current = null;
      }
    },
    [actor, thread],
  );
  const end = useCallback(async () => {
    const previous = client.current;
    client.current = null;
    generation.current++;
    setHolding(false);
    setStarting(false);
    setConnected(false);
    setInputAccepting(false);
    setConnectionStatus(null);
    setSnapshot(null);
    setActivity((previous) => ({
      ...silentActivity(),
      playbackMs: previous.playbackMs,
    }));
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
    setInputAccepting(false);
    const next = new AssistantVoiceClient(
      thread,
      (value) => {
        if (generation.current === ticket) {
          setConflictSession(null);
          observeSession(value.session.id);
          setSnapshot(value);
          if (value.session.state === "closing") {
            setStarting(false);
            setConnected(true);
            setHolding(false);
          }
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
          setInputAccepting(false);
          setHolding(false);
          setConnectionStatus(null);
        }
      },
      (session) => {
        if (generation.current === ticket) setConflictSession(session);
      },
      undefined,
      (status) => {
        if (generation.current === ticket) {
          observeStatus(status);
          setConnectionStatus(status);
          setStarting(status !== "ready");
          setConnected(status === "ready");
          if (status !== "ready") {
            setHolding(false);
            setInputAccepting(false);
          }
        }
      },
      (enabled) => {
        if (generation.current === ticket) setInputAccepting(enabled);
      },
    );
    client.current = next;
    setError(null);
    setSnapshot(null);
    next.rejoin(conflictSession);
  }, [conflictSession, thread]);
  return {
    activity,
    inputAccepting,
    restoredEvent,
    holding,
    connected,
    snapshot,
    starting,
    connectionStatus,
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
      if (pressed && (starting || !connected)) return;
      setHolding(pressed);
      client.current?.hold(pressed);
    },
    mute: (muted: boolean) => client.current?.mute(muted),
    resumeAudio: () => client.current?.resumeAudio(),
    stopTask: (id: string) => client.current?.stopTask(id),
  };
}
