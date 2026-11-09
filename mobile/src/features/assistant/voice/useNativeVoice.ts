import { waitForVoiceForeground } from "./voicePermission";
import { useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { Camera } from "expo-camera";
import * as Crypto from "expo-crypto";
import { getAuthEpoch } from "../../../lib/auth/authEpoch";
import type { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { AssistantVoiceClient } from "./AssistantVoiceClient";
import { NativeVoiceMedia } from "./VoiceMediaBridge";
import type { VoiceStartRecovery } from "./voiceStartError";
import { discoverActiveVoiceCall } from "./voiceActiveCall";
import { voiceSessionSchema } from "./voiceContracts";
import type {
  VoicePreferences,
  VoiceProtocol,
  VoiceSession,
  VoiceSnapshot,
} from "./voiceContracts";

type Environment = ReturnType<typeof nativeVoiceEnvironment>;

type RecoveryTarget = {
  session: VoiceSession | null;
  thread?: string;
  unknown: boolean;
  pendingStart?: boolean;
};
// Only safe session metadata survives the surface. A new account or explicit
// recovery action fences late completions from an older client.
let retainedRecovery:
  | (RecoveryTarget & { epoch: number; owner: object })
  | undefined;
const recoveryListeners = new Set<() => void>();

export function useNativeVoice(
  thread: string,
  environment: Environment,
  onUpdate: () => void,
) {
  const [media, setMedia] = useState<NativeVoiceMedia | null>(null);
  const [mediaAttempt, setMediaAttempt] = useState(0);
  const [starting, setStarting] = useState(false);
  const [connected, setConnected] = useState(false);
  const [inputEnabled, setInputEnabled] = useState(false);
  const [holding, setHolding] = useState(false);
  const [speakerMuted, setSpeakerMuted] = useState(false);
  const [snapshot, setSnapshot] = useState<VoiceSnapshot | null>(null);
  const [error, setError] = useState<string>();
  const [recovery, setRecovery] = useState<VoiceStartRecovery>();
  const [conflict, setConflict] = useState<VoiceSession | null>(null);
  const [conflictThread, setConflictThread] = useState<string>();
  const [conflictUnknown, setConflictUnknown] = useState(false);
  const [endingConflict, setEndingConflict] = useState(false);
  const [checkingConflict, setCheckingConflict] = useState(false);
  const [confirmedEnded, setConfirmedEnded] = useState(false);
  const [conflictCheckAttempt, setConflictCheckAttempt] = useState(0);
  const current = useRef<AssistantVoiceClient | null>(null);
  const clientThread = useRef(thread);
  const ownership = useRef<object | undefined>(undefined);
  const scope = useRef({ thread, environment });
  scope.current = { thread, environment };
  const latestSnapshot = useRef<VoiceSnapshot | null>(null);
  const taskCancellations = useRef(new Map<string, AbortController>());
  const ticket = useRef(0);
  const mounted = useRef(true);
  const busy = useRef(false);
  const conflictBusy = useRef(false);
  const conflictChecks = useRef(0);
  const latestConflict = useRef({
    session: conflict,
    thread: conflictThread,
    unknown: conflictUnknown,
  });
  latestConflict.current = {
    session: conflict,
    thread: conflictThread,
    unknown: conflictUnknown,
  };
  const permissionAttempt = useRef<number | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  const update = useRef(onUpdate);
  update.current = onUpdate;
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );

  // Native gestures and async work can retain a handler from an earlier chat.
  // Check its scope before touching the shared client ref or making a request.
  function sameChat() {
    return (
      mounted.current &&
      scope.current.thread === thread &&
      scope.current.environment === environment
    );
  }
  function sameScope() {
    return sameChat() && environment.epoch === getAuthEpoch();
  }
  // Silencing local audio is always safe for this chat, even after the account
  // changed underneath it; only actions that open audio or reach the server
  // need the full scope.
  const quieting = (open: boolean) => (open ? sameScope() : sameChat());

  function reportError(message?: string, nextRecovery?: VoiceStartRecovery) {
    setError(message);
    setRecovery(nextRecovery);
  }

  function claimRecovery() {
    const owner = {};
    ownership.current = owner;
    retainedRecovery = {
      epoch: environment.epoch,
      owner,
      session: null,
      unknown: false,
    };
    return owner;
  }

  function remember(target: RecoveryTarget, owner = ownership.current) {
    if (
      environment.epoch !== getAuthEpoch() ||
      !owner ||
      retainedRecovery?.epoch !== environment.epoch ||
      retainedRecovery.owner !== owner
    )
      return;
    retainedRecovery = { ...target, epoch: environment.epoch, owner };
    recoveryListeners.forEach((listener) => listener());
  }

  function refresh() {
    if (!sameScope() || refreshTimer.current) return;
    refreshTimer.current = setTimeout(() => {
      refreshTimer.current = undefined;
      if (sameScope()) update.current();
    }, 1000);
  }

  function stopLocal() {
    ticket.current++;
    pending.current?.abort();
    for (const controller of taskCancellations.current.values())
      controller.abort();
    taskCancellations.current.clear();
    permissionAttempt.current = undefined;
    busy.current = false;
    conflictBusy.current = false;
    const previous = current.current;
    current.current = null;
    const cleanup = previous?.end();
    if (previous?.pendingEnd)
      remember({
        session: previous.pendingEnd,
        thread: clientThread.current,
        unknown: false,
      });
    else if (previous?.pendingStart)
      remember({ session: null, unknown: true, pendingStart: true });
    media?.close();
    if (mounted.current) {
      setStarting(false);
      setConnected(false);
      setInputEnabled(false);
      setHolding(false);
      setEndingConflict(false);
      setCheckingConflict(false);
    }
    return cleanup;
  }

  const stopRef = useRef(stopLocal);
  stopRef.current = stopLocal;
  useEffect(() => {
    mounted.current = true;
    busy.current = false;
    conflictBusy.current = false;
    setMedia(null);
    latestSnapshot.current = null;
    setSnapshot(null);
    setConfirmedEnded(false);
    if (retainedRecovery?.epoch !== environment.epoch)
      retainedRecovery = undefined;
    ownership.current = retainedRecovery?.owner;
    const restoreRecovery = () => {
      if (!sameScope()) return;
      const target =
        retainedRecovery?.epoch === environment.epoch
          ? retainedRecovery
          : undefined;
      latestConflict.current = {
        session: target?.session ?? null,
        thread: target?.thread,
        unknown: target?.unknown ?? false,
      };
      if (target?.session || target?.unknown) setConfirmedEnded(false);
      setConflict(target?.session ?? null);
      setConflictThread(target?.thread);
      setConflictUnknown(target?.unknown ?? false);
    };
    restoreRecovery();
    recoveryListeners.add(restoreRecovery);
    conflictChecks.current = 0;
    reportError();
    setConnected(false);
    setInputEnabled(false);
    setStarting(false);
    setHolding(false);
    setSpeakerMuted(false);
    setEndingConflict(false);
    setCheckingConflict(false);
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        // A scheduled check may have fired while backgrounded. Resume only
        // the remaining bounded checks; foreground never starts or ends a call.
        if (sameScope()) setConflictCheckAttempt((attempt) => attempt + 1);
        return;
      }
      // System permission prompts may briefly inactivate the app before any
      // microphone is acquired. Actual background always cancels the attempt.
      if (
        !(state === "inactive" && permissionAttempt.current === ticket.current)
      ) {
        const hadVoiceWork = busy.current || current.current !== null;
        stopRef.current();
        if (hadVoiceWork)
          reportError(
            "Voice ended while the app was inactive. Start a new call to continue.",
          );
      }
    });
    return () => {
      mounted.current = false;
      recoveryListeners.delete(restoreRecovery);
      subscription.remove();
      stopRef.current();
      clearTimeout(refreshTimer.current);
      refreshTimer.current = undefined;
    };
  }, [thread, environment]);

  useEffect(() => {
    const ownedDisconnected =
      !starting &&
      !connected &&
      !confirmedEnded &&
      snapshot &&
      snapshot.session.state !== "closed" &&
      snapshot.session.state !== "failed" &&
      (!conflict ||
        (conflict.id === snapshot.session.id && conflictThread === thread));
    if (
      (!(conflict?.end_requested || conflict?.state === "closing") &&
        !ownedDisconnected) ||
      conflictChecks.current >= 6
    )
      return;
    const timer = setTimeout(() => {
      if (AppState.currentState === "active" && !conflictBusy.current) {
        conflictChecks.current++;
        void checkConflict(false);
      }
    }, 1000);
    return () => clearTimeout(timer);
  }, [
    conflict,
    conflictThread,
    conflictCheckAttempt,
    starting,
    connected,
    confirmedEnded,
    snapshot,
  ]);

  async function start(preferences: VoicePreferences, protocol: VoiceProtocol) {
    if (
      !sameScope() ||
      AppState.currentState !== "active" ||
      busy.current ||
      current.current ||
      conflict ||
      conflictUnknown ||
      latestConflict.current.session ||
      latestConflict.current.unknown ||
      (retainedRecovery?.epoch === environment.epoch &&
        (retainedRecovery.session || retainedRecovery.unknown)) ||
      conflictBusy.current
    )
      return;
    busy.current = true;
    const generation = ++ticket.current;
    const controller = new AbortController();
    pending.current = controller;
    const valid = () =>
      sameScope() &&
      ticket.current === generation &&
      !controller.signal.aborted;
    setStarting(true);
    setConfirmedEnded(false);
    setInputEnabled(false);
    reportError();
    latestSnapshot.current = null;
    setSnapshot(null);
    setSpeakerMuted(false);
    try {
      const options = await environment.api.options(thread, controller.signal);
      if (!valid()) return;
      const selected = options.options.find(
        (option) =>
          option.service_id === preferences.service_id &&
          option.connection_id === preferences.connection_id &&
          option.key_source === preferences.key_source &&
          option.model === preferences.model &&
          option.voice.protocol === protocol,
      );
      if (!selected?.available)
        throw new Error(
          selected?.unavailable_reason ??
            "This voice source is no longer available.",
        );
      await environment.api.savePreferences(preferences, controller.signal);
      if (!valid()) return;
      permissionAttempt.current = generation;
      let permission;
      try {
        permission = await Camera.requestMicrophonePermissionsAsync();
        if (!valid()) return;
        if (!permission.granted)
          throw new Error(
            "Microphone access is off. Allow it in your device settings to use voice; you can keep chatting with text.",
          );
        await waitForVoiceForeground(AppState, controller.signal, valid);
      } finally {
        if (permissionAttempt.current === generation)
          permissionAttempt.current = undefined;
      }
      if (!valid()) return;
      const nextMedia = new NativeVoiceMedia();
      const owner = claimRecovery();
      let connectedOnce = false;
      const next = new AssistantVoiceClient({
        conversationId: thread,
        media: nextMedia,
        api: environment.api,
        createSocket: environment.createSocket,
        randomId: Crypto.randomUUID,
        onSnapshot: (value) => {
          if (!valid()) return;
          const previous = latestSnapshot.current;
          latestSnapshot.current = value;
          if (
            connectedOnce &&
            value.session.state === "closed" &&
            value.session.end_reason === "idle"
          )
            reportError();
          if (
            value.session.state !== "active" ||
            value.session.muted ||
            value.session.input_muted ||
            value.session.end_requested ||
            (previous &&
              previous.session.generation !== value.session.generation)
          )
            setHolding(false);
          setSnapshot(value);
          refresh();
        },
        onReady: () => {
          if (!valid() || current.current !== next) return;
          connectedOnce = true;
          setStarting(false);
          setConnected(true);
        },
        onInputChanged: (enabled) => {
          if (valid() && current.current === next) setInputEnabled(enabled);
        },
        onError: (message, nextRecovery) => {
          if (valid()) reportError(message, nextRecovery);
        },
        onConflict: (session, conversationId, unknown = false) => {
          if (!valid()) return;
          remember(
            {
              session,
              thread: session ? (conversationId ?? thread) : undefined,
              unknown,
            },
            owner,
          );
          conflictChecks.current = 0;
        },
        onCleanup: (session, unknown, conversationId) => {
          remember(
            {
              session,
              thread: session ? (conversationId ?? thread) : undefined,
              unknown,
              pendingStart: !session && next.pendingStart,
            },
            owner,
          );
        },
        onClosed: () => {
          if (!valid()) return;
          if (current.current === next) current.current = null;
          if (next.pendingEnd) {
            remember(
              { session: next.pendingEnd, thread, unknown: false },
              owner,
            );
            conflictChecks.current = 0;
          }
          setStarting(false);
          setConnected(false);
          setInputEnabled(false);
          setHolding(false);
          refresh();
        },
      });
      current.current = next;
      clientThread.current = thread;
      setMedia(nextMedia);
      setMediaAttempt(generation);
      await next.start(preferences, protocol);
      if (valid() && next.isClosed) current.current = null;
    } catch (cause) {
      if (valid())
        reportError(
          cause &&
            typeof cause === "object" &&
            "message" in cause &&
            typeof cause.message === "string"
            ? cause.message
            : "Voice could not start. Try again.",
        );
    } finally {
      if (valid()) {
        busy.current = false;
        if (!current.current) setStarting(false);
      }
    }
  }

  async function end(): Promise<boolean> {
    if (!sameChat()) return false;
    const previous = current.current;
    const cleanup = stopLocal();
    const generation = ticket.current;
    if (sameScope() && previous?.pendingEnd) {
      remember({ session: previous.pendingEnd, thread, unknown: false });
      conflictChecks.current = 0;
    }
    await cleanup;
    if (sameScope() && ticket.current === generation && previous) {
      if (!previous.pendingStart)
        remember({
          session: previous.pendingEnd ?? null,
          thread: previous.pendingEnd ? thread : undefined,
          unknown: false,
        });
      conflictChecks.current = 0;
    }
    refresh();
    return (
      sameScope() &&
      ticket.current === generation &&
      !previous?.pendingStart &&
      !latestConflict.current.session &&
      !latestConflict.current.unknown
    );
  }

  async function checkConflict(manual = true): Promise<boolean> {
    if (
      !sameScope() ||
      AppState.currentState !== "active" ||
      conflictBusy.current ||
      current.current ||
      busy.current
    )
      return false;
    if (
      retainedRecovery?.epoch === environment.epoch &&
      retainedRecovery.pendingStart
    ) {
      reportError(
        "The previous voice start is still settling. Check again shortly.",
      );
      return false;
    }
    conflictBusy.current = true;
    const generation = ++ticket.current;
    const controller = new AbortController();
    pending.current = controller;
    const target = latestConflict.current;
    const localSession = latestSnapshot.current?.session;
    // A completed client cleanup can clear recovery after local End has
    // fenced its final snapshot. Retain the owned snapshot's exact identity
    // until a read confirms terminal state or an empty active-call result.
    const priorTarget =
      !target.session &&
      localSession &&
      localSession.state !== "closed" &&
      localSession.state !== "failed"
        ? { ...target, session: localSession, thread }
        : target;
    const owner = claimRecovery();
    remember(priorTarget, owner);
    const valid = () =>
      sameScope() &&
      ticket.current === generation &&
      !controller.signal.aborted;
    setCheckingConflict(true);
    setConfirmedEnded(false);
    if (manual) {
      conflictChecks.current = 0;
      reportError();
    }
    try {
      if (priorTarget.session && priorTarget.thread) {
        try {
          const prior = voiceSessionSchema.parse(
            await environment.api.status(
              priorTarget.thread,
              priorTarget.session.id,
              controller.signal,
            ),
          );
          if (!valid()) return false;
          const previous = latestSnapshot.current;
          const observed = [
            priorTarget.session,
            ...(priorTarget.thread === thread &&
            previous?.session.id === prior.id
              ? [previous.session]
              : []),
          ];
          const currentStatus = observed.every(
            (value) =>
              prior.id === value.id &&
              (prior.generation > value.generation ||
                (prior.generation === value.generation &&
                  prior.control_revision >= value.control_revision)),
          );
          if (
            currentStatus &&
            (prior.state === "closed" || prior.state === "failed")
          ) {
            // An owned call's terminal receipt does not depend on optional
            // account discovery or a bounded scan of other conversations.
            // A later explicit Start still passes backend account admission.
            remember({ session: null, unknown: false }, owner);
            setConfirmedEnded(true);
            if (
              priorTarget.thread === thread &&
              (!previous || previous.session.id === prior.id)
            ) {
              const terminal: VoiceSnapshot = {
                type: "snapshot",
                session: prior,
                captions: previous?.captions ?? [],
                tasks: previous?.tasks ?? [],
              };
              latestSnapshot.current = terminal;
              setSnapshot(terminal);
            }
            reportError();
            refresh();
            return true;
          }
        } catch {
          if (!valid()) return false;
          // Discovery can still identify a newer call or prove an empty slot.
        }
      }
      const call = await discoverActiveVoiceCall(
        environment.api,
        priorTarget.thread ?? thread,
        controller.signal,
      );
      if (!valid()) return false;
      remember(
        {
          session: call?.session ?? null,
          thread: call?.conversationId,
          unknown: false,
        },
        owner,
      );
      setConfirmedEnded(!call);
      if (!call) reportError();
      return !call;
    } catch {
      if (valid()) {
        remember({ ...priorTarget, unknown: true }, owner);
        reportError(
          "Call status could not be verified. Check status again before starting voice.",
        );
      }
      return false;
    } finally {
      if (valid()) {
        conflictBusy.current = false;
        setCheckingConflict(false);
        // Even a repeated failure may preserve the same session object. Rearm
        // bounded polling independently of session identity or error text.
        setConflictCheckAttempt((attempt) => attempt + 1);
      }
    }
  }

  async function endConflict(): Promise<boolean> {
    if (
      !sameScope() ||
      !conflict ||
      conflictBusy.current ||
      conflict.id !== latestConflict.current.session?.id ||
      conflictThread !== latestConflict.current.thread
    )
      return false;
    conflictBusy.current = true;
    const generation = ++ticket.current;
    const owner = claimRecovery();
    remember(
      { session: conflict, thread: conflictThread ?? thread, unknown: false },
      owner,
    );
    const controller = new AbortController();
    pending.current = controller;
    const valid = () =>
      sameScope() &&
      ticket.current === generation &&
      !controller.signal.aborted;
    setEndingConflict(true);
    reportError();
    try {
      const next = voiceSessionSchema.parse(
        await environment.api.control(conflictThread ?? thread, conflict.id, {
          command_id: Crypto.randomUUID(),
          expected_revision: conflict.control_revision,
          action: "end",
        }),
      );
      if (next.id !== conflict.id) throw new Error("Voice session changed");
      if (valid()) {
        const terminal = next.state === "closed" || next.state === "failed";
        remember(
          {
            session: terminal ? null : next,
            thread: terminal ? undefined : (conflictThread ?? thread),
            unknown: false,
          },
          owner,
        );
        conflictChecks.current = 0;
        refresh();
        return terminal;
      }
      return false;
    } catch {
      if (!valid()) return false;
      reportError(
        "The existing call could not be ended. Refresh its state and try again.",
      );
      try {
        const active = await discoverActiveVoiceCall(
          environment.api,
          conflictThread ?? thread,
          controller.signal,
        );
        if (valid()) {
          remember(
            {
              session: active?.session ?? null,
              thread: active?.conversationId,
              unknown: false,
            },
            owner,
          );
          return !active;
        }
      } catch {
        /* Keep the last verified conflict; do not create another call. */
      }
      return false;
    } finally {
      if (valid()) {
        conflictBusy.current = false;
        setEndingConflict(false);
      }
    }
  }

  async function stopTask(id: string) {
    if (!sameScope()) return;
    if (current.current) return current.current.stopTask(id);
    const task = latestSnapshot.current?.tasks.find((row) => row.id === id);
    if (
      !task ||
      (task.state !== "queued" && task.state !== "claimed") ||
      taskCancellations.current.has(id)
    )
      return;
    const controller = new AbortController();
    const generation = ticket.current;
    const valid = () =>
      sameScope() &&
      ticket.current === generation &&
      !controller.signal.aborted;
    taskCancellations.current.set(id, controller);
    try {
      await environment.api.cancelRequest(thread, id, controller.signal);
      if (valid()) refresh();
    } catch {
      if (valid())
        reportError("The task could not be stopped. Try again from the chat.");
    } finally {
      if (taskCancellations.current.get(id) === controller)
        taskCancellations.current.delete(id);
    }
  }

  return {
    media,
    mediaAttempt,
    starting,
    connected,
    inputEnabled,
    snapshot,
    holding,
    speakerMuted,
    error,
    recovery,
    conflict,
    conflictThread,
    conflictUnknown,
    endingConflict,
    checkingConflict,
    confirmedEnded,
    start,
    end,
    endConflict,
    checkConflict,
    hold(pressed: boolean) {
      if (!quieting(pressed)) return;
      setHolding(current.current?.hold(pressed) ?? false);
    },
    mute(muted: boolean) {
      if (!quieting(!muted)) return;
      setHolding(false);
      return current.current?.mute(muted);
    },
    muteSpeaker(muted: boolean) {
      if (!quieting(!muted)) return;
      current.current?.muteSpeaker(muted);
      setSpeakerMuted(muted);
    },
    /** Change the listening mode of a live call; false when it cannot. */
    setInputMode(mode: VoicePreferences["input_mode"]) {
      if (!sameScope()) return false;
      setHolding(false);
      return current.current?.setInputMode(mode) ?? false;
    },
    resumeAudio() {
      if (!sameScope()) return;
      if (
        current.current &&
        error === "Tap Resume audio to hear the assistant."
      )
        reportError();
      return current.current?.resumeAudio();
    },
    stopTask,
  };
}
