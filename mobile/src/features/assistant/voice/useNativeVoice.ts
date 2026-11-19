import { useLocalizedState } from "../../../i18n/useLocalizedState";
import { localizedError } from "../../../i18n/localizedError";
import { translateApp } from "../../../i18n/appTranslation";
import { useAppLanguage } from "../../../i18n/LanguageProvider";
import { waitForVoiceForeground } from "./voicePermission";
import { useEffect, useRef, useState } from "react";
import { AppState, Linking, Platform } from "react-native";
import { Camera } from "expo-camera";
import * as Crypto from "expo-crypto";
import { getAuthEpoch } from "../../../lib/auth/authEpoch";
import type { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { AssistantVoiceClient } from "./AssistantVoiceClient";
import { NativeVoiceMedia } from "./VoiceMediaBridge";
import type { VoiceStartRecovery } from "./voiceStartError";
import { discoverActiveVoiceCall } from "./voiceActiveCall";
import { voiceSessionSchema } from "./voiceContracts";
import { withVoiceRequestDeadline } from "./voiceRequestDeadline";
import type {
  VoicePreferences,
  VoiceOption,
  VoiceProtocol,
  VoiceSession,
  VoiceSnapshot,
} from "./voiceContracts";

type Environment = ReturnType<typeof nativeVoiceEnvironment>;
const MICROPHONE_PERMISSION_TIMEOUT_MS = 60_000;
// 30s heartbeat expiry + the server's 7s close budget, with 8s scheduling margin.
export const OWNED_CALL_RECOVERY_MS = 45_000;
type MicrophonePermission = { kind: "denied"; canAskAgain: boolean };
type PermissionAttempt = {
  generation: number;
  androidBackgroundAllowed: boolean;
  androidBackgrounded: boolean;
  requestTimer?: ReturnType<typeof setTimeout>;
};

type RecoveryTarget = {
  session: VoiceSession | null;
  thread?: string;
  unknown: boolean;
  pendingStart?: boolean;
  endedHere?: boolean;
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
  const { t } = useAppLanguage();
  const translate = useRef(t);
  translate.current = t;
  const [media, setMedia] = useState<NativeVoiceMedia | null>(null);
  const [mediaAttempt, setMediaAttempt] = useState(0);
  const [starting, setStarting] = useState(false);
  const [connected, setConnected] = useState(false);
  const [connectedAt, setConnectedAt] = useState<number>();
  const [disconnectedAt, setDisconnectedAt] = useState<number>();
  const [inputEnabled, setInputEnabled] = useState(false);
  const [holding, setHolding] = useState(false);
  const [speakerMuted, setSpeakerMuted] = useState(false);
  const [snapshot, setSnapshot] = useState<VoiceSnapshot | null>(null);
  const [error, setError] = useLocalizedState<string>();
  const [microphonePermission, setMicrophonePermission] =
    useState<MicrophonePermission>();
  const permissionState = useRef(microphonePermission);
  permissionState.current = microphonePermission;
  const nativeConnected = useRef(false);
  const recoveryStarted = useRef<number | undefined>(undefined);
  const [recovery, setRecovery] = useState<VoiceStartRecovery>();
  const [conflict, setConflict] = useState<VoiceSession | null>(null);
  const [conflictThread, setConflictThread] = useState<string>();
  const [conflictUnknown, setConflictUnknown] = useState(false);
  const [endingConflict, setEndingConflict] = useState(false);
  const [checkingConflict, setCheckingConflict] = useState(false);
  const [confirmedEnded, setConfirmedEnded] = useState(false);
  const [conflictCheckAttempt, setConflictCheckAttempt] = useState(0);
  const current = useRef<AssistantVoiceClient | null>(null);
  const draining = useRef<Promise<void> | undefined>(undefined);
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
  const permissionAttempt = useRef<PermissionAttempt | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  const update = useRef(onUpdate);
  const errorOrigin = useRef<"call" | "status" | "playback" | undefined>(
    undefined,
  );
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

  function reportError(
    message?: string | (() => string),
    nextRecovery?: VoiceStartRecovery,
    origin: "call" | "status" | "playback" = "call",
  ) {
    errorOrigin.current = message || nextRecovery ? origin : undefined;
    setError(typeof message === 'function' ? message : message === undefined ? undefined : () => translateApp(message));
    setRecovery(nextRecovery);
  }

  function reportStatusError(message?: string | (() => string)) {
    if (errorOrigin.current === "call") return;
    reportError(message, undefined, "status");
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
    clearTimeout(permissionAttempt.current?.requestTimer);
    permissionAttempt.current = undefined;
    busy.current = false;
    conflictBusy.current = false;
    const previous = current.current;
    if (previous) {
      recoveryStarted.current = Date.now();
      conflictChecks.current = 0;
    }
    nativeConnected.current = false;
    current.current = null;
    // The client silences capture/output synchronously and owns the bounded
    // transport drain. Closing its media here would cut off session.closed.
    const cleanup = previous?.end() ?? draining.current;
    if (cleanup) {
      draining.current = cleanup;
      void cleanup
        .finally(() => {
          if (draining.current !== cleanup) return;
          draining.current = undefined;
          if (sameScope()) setConflictCheckAttempt((attempt) => attempt + 1);
        })
        .catch(() => {});
    }
    if (previous?.pendingEnd)
      remember({
        session: previous.pendingEnd,
        endedHere: true,
        thread: clientThread.current,
        unknown: false,
      });
    else if (previous?.pendingStart)
      remember({ session: null, unknown: true, pendingStart: true });
    if (!previous && !cleanup) media?.close();
    if (mounted.current) {
      setStarting(false);
      setConnected(false);
      setDisconnectedAt((previous) => previous ?? Date.now());
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
    setMicrophonePermission(undefined);
    nativeConnected.current = false;
    busy.current = false;
    conflictBusy.current = false;
    setMedia(null);
    latestSnapshot.current = null;
    setSnapshot(null);
    setConnectedAt(undefined);
    setDisconnectedAt(undefined);
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
    let backgrounded = AppState.currentState === "background";
    const subscription = AppState.addEventListener("change", (state) => {
      // iOS alerts and overlays report inactive without leaving the foreground.
      if (state === "inactive") return;
      const permission = permissionAttempt.current;
      const awaitingPermission = permission?.generation === ticket.current;
      if (state === "active") {
        if (!backgrounded) {
          // Resume a paused recovery timer without resyncing a live call or
          // granting a fresh recovery budget for an overlay.
          const session = latestSnapshot.current?.session;
          if (
            sameScope() &&
            (latestConflict.current.session ||
              latestConflict.current.unknown ||
              (!current.current &&
                session &&
                session.state !== "closed" &&
                session.state !== "failed"))
          )
            setConflictCheckAttempt((attempt) => attempt + 1);
          return;
        }
        backgrounded = false;
        if (permission?.androidBackgrounded) {
          // A subsequent background event is no longer the permission prompt.
          permission.androidBackgroundAllowed = false;
          permission.androidBackgrounded = false;
        }
        if (sameScope() && permissionState.current) {
          const generation = ticket.current;
          void Camera.getMicrophonePermissionsAsync()
            .then((permission) => {
              if (
                !sameScope() ||
                ticket.current !== generation ||
                current.current
              )
                return;
              if (permission.granted) {
                setMicrophonePermission(undefined);
                reportError();
              } else {
                setMicrophonePermission({
                  kind: "denied",
                  canAskAgain: permission.canAskAgain !== false,
                });
              }
            })
            .catch(() => {});
        }
        const client = current.current;
        if (client && nativeConnected.current) {
          const generation = ticket.current;
          void client.resync().then((result) => {
            if (
              !sameScope() ||
              current.current !== client ||
              ticket.current !== generation ||
              result !== "ended"
            )
              return;
            stopRef.current();
            reportError(
              () => (translateApp(
                "The call ended while NyxID was in the background. Start a new call.",
              )),
            );
          });
        }
        if (retainedRecovery?.endedHere) {
          recoveryStarted.current = Date.now();
          conflictChecks.current = 0;
        }
        // Foreground resumes exact-session recovery; it never creates a call.
        if (sameScope()) setConflictCheckAttempt((attempt) => attempt + 1);
        return;
      }
      if (state !== "background") return;
      backgrounded = true;
      // Android's permission activity reports background before capture starts.
      if (
        permission &&
        awaitingPermission &&
        permission.androidBackgroundAllowed &&
        !current.current
      ) {
        permission.androidBackgrounded = true;
        return;
      }
      if (
        Platform.OS === "ios" &&
        nativeConnected.current &&
        current.current &&
        latestSnapshot.current?.session.state === "active" &&
        !latestSnapshot.current.session.end_requested
      )
        return;
      const wasConnecting =
        busy.current || (!nativeConnected.current && !latestSnapshot.current);
      const hadVoiceWork = busy.current || current.current !== null;
      stopRef.current();
      if (hadVoiceWork)
        reportError(
          () => (translateApp(
            wasConnecting
              ? "The call ended because NyxID went to the background while it was still connecting."
              : Platform.OS === "android"
                ? "The call ended because NyxID went to the background. Android can't keep the microphone on in the background."
                : "The call ended because this voice service can't keep the microphone on in the background.",
          )),
        );
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
    const ownedRecovery = !!retainedRecovery?.endedHere || !!ownedDisconnected;
    if (
      (!conflict && !conflictUnknown && !ownedDisconnected) ||
      (ownedRecovery
        ? recoveryStarted.current !== undefined &&
          Date.now() - recoveryStarted.current >= OWNED_CALL_RECOVERY_MS
        : conflictChecks.current >= 6)
    )
      return;
    recoveryStarted.current ??= Date.now();
    const timer = setTimeout(
      () => {
        if (
          AppState.currentState === "active" &&
          (!ownedRecovery ||
            Date.now() - recoveryStarted.current! < OWNED_CALL_RECOVERY_MS) &&
          !conflictBusy.current &&
          !draining.current
        ) {
          conflictChecks.current++;
          void checkConflict(false);
        }
      },
      ownedRecovery
        ? Math.min(500 * 2 ** Math.min(conflictChecks.current, 3), 4000)
        : 500,
    );
    return () => clearTimeout(timer);
  }, [
    conflict,
    conflictThread,
    conflictUnknown,
    conflictCheckAttempt,
    starting,
    connected,
    confirmedEnded,
    snapshot,
  ]);

  async function start(
    preferences: VoicePreferences,
    protocol: VoiceProtocol,
    savedPreferences?: VoicePreferences | null,
    selectedOption?: VoiceOption,
  ) {
    // A new explicit attempt can wait for the old client's bounded teardown.
    // Recheck all chat/account guards after that asynchronous boundary.
    if (!sameScope()) return false;
    const waitingTicket = ticket.current;
    if (draining.current) {
      await draining.current;
      if (ticket.current !== waitingTicket) return false;
    }
    if (
      !sameScope() ||
      AppState.currentState !== "active" ||
      busy.current ||
      draining.current ||
      current.current ||
      conflict ||
      conflictUnknown ||
      latestConflict.current.session ||
      latestConflict.current.unknown ||
      (retainedRecovery?.epoch === environment.epoch &&
        (retainedRecovery.session || retainedRecovery.unknown)) ||
      conflictBusy.current
    )
      return false;
    nativeConnected.current = false;
    busy.current = true;
    const generation = ++ticket.current;
    const controller = new AbortController();
    pending.current = controller;
    const ownsOperation = () => sameScope() && ticket.current === generation;
    const valid = () => ownsOperation() && !controller.signal.aborted;
    setStarting(true);
    setConnectedAt(undefined);
    setDisconnectedAt(undefined);
    setConfirmedEnded(false);
    setInputEnabled(false);
    reportError();
    latestSnapshot.current = null;
    setSnapshot(null);
    setSpeakerMuted(false);
    try {
      // The call surface has already loaded this chat's choices. Reuse that
      // selection; POST admission still resolves live credentials and authority.
      const options = selectedOption
        ? { options: [selectedOption] }
        : await withVoiceRequestDeadline(controller, () =>
            environment.api.options(thread, controller.signal),
          );
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
            translateApp("This voice source is no longer available."),
        );
      if (
        !savedPreferences ||
        (Object.keys(preferences) as (keyof VoicePreferences)[]).some(
          (key) => preferences[key] !== savedPreferences[key],
        )
      )
        await withVoiceRequestDeadline(controller, () =>
          environment.api.savePreferences(preferences, controller.signal),
        );
      if (!valid()) return;
      let permission = await withVoiceRequestDeadline(controller, () =>
        Camera.getMicrophonePermissionsAsync(),
      );
      if (!valid()) return;
      if (!permission.granted && permission.canAskAgain === false) {
        setMicrophonePermission({ kind: "denied", canAskAgain: false });
        throw new Error(
          translate.current(
            "Microphone access is off. Allow it in your device settings to use voice; you can keep chatting with text.",
          ),
        );
      }
      if (!permission.granted) {
        const attempt: PermissionAttempt = {
          generation,
          androidBackgroundAllowed: Platform.OS === "android",
          androidBackgrounded: false,
        };
        permissionAttempt.current = attempt;
        try {
          // Give people time to read the system prompt. The shorter foreground
          // handoff timeout starts only once they have answered it.
          attempt.requestTimer = setTimeout(() => {
            if (permissionAttempt.current !== attempt) return;
            stopRef.current();
            reportError(
              () => (translateApp(
                "Microphone permission took too long. Try voice again and respond to the permission prompt.",
              )),
            );
          }, MICROPHONE_PERMISSION_TIMEOUT_MS);
          permission = await Camera.requestMicrophonePermissionsAsync();
          clearTimeout(attempt.requestTimer);
          attempt.requestTimer = undefined;
          if (!valid()) return;
          if (!permission.granted) {
            setMicrophonePermission({
              kind: "denied",
              canAskAgain: permission.canAskAgain !== false,
            });
            throw new Error(
              translate.current(
                "Microphone access is off. Allow it in your device settings to use voice; you can keep chatting with text.",
              ),
            );
          }
          await waitForVoiceForeground(AppState, controller.signal, valid, {
            allowInitialBackground: attempt.androidBackgroundAllowed,
            waitWhileInactive: Platform.OS === "ios",
          });
        } finally {
          clearTimeout(attempt.requestTimer);
          if (permissionAttempt.current === attempt) {
            permissionAttempt.current = undefined;
          }
        }
      }
      if (!valid()) return;
      if (Platform.OS === "ios") {
        do {
          await waitForVoiceForeground(AppState, controller.signal, valid, {
            waitWhileInactive: true,
          });
          if (!valid()) return;
        } while (AppState.currentState !== "active");
      }
      if (AppState.currentState !== "active")
        throw new Error(
          translate.current("Keep NyxID open to start a voice call."),
        );
      setMicrophonePermission(undefined);
      nativeConnected.current = false;
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
            (errorOrigin.current === "playback" &&
              (value.session.state === "closed" ||
                value.session.state === "failed")) ||
            (connectedOnce &&
              value.session.state === "closed" &&
              value.session.end_reason === "idle")
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
          if (
            value.session.state === "closed" ||
            value.session.state === "failed"
          )
            setDisconnectedAt((previous) => previous ?? Date.now());
          setSnapshot(value);
          refresh();
        },
        onReady: () => {
          if (!valid() || current.current !== next) return;
          connectedOnce = true;
          nativeConnected.current =
            Platform.OS === "ios" && nextMedia.usesNativeMedia;
          // The call clock starts when the provider is truly ready, not at Start.
          setConnectedAt((previous) => previous ?? Date.now());
          setStarting(false);
          setConnected(true);
        },
        onInputChanged: (enabled) => {
          if (valid() && current.current === next) setInputEnabled(enabled);
        },
        onError: (message, nextRecovery, kind, source) => {
          if (valid())
            reportError(
              source ?? (() => translateApp(message)),
              nextRecovery,
              kind === "conflict" ? "status" : (kind ?? "call"),
            );
        },
        onConflict: (session, conversationId, unknown = false) => {
          if (!valid()) return;
          remember(
            {
              session,
              thread: session ? (conversationId ?? thread) : undefined,
              unknown,
              endedHere: !!session && next.pendingEnd?.id === session.id,
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
              endedHere: !!session && next.pendingEnd?.id === session.id,
            },
            owner,
          );
        },
        onClosed: () => {
          if (!valid()) return;
          nativeConnected.current = false;
          if (current.current === next) current.current = null;
          if (next.pendingEnd) {
            remember(
              {
                session: next.pendingEnd,
                thread,
                unknown: false,
                endedHere: true,
              },
              owner,
            );
            conflictChecks.current = 0;
          }
          setStarting(false);
          setConnected(false);
          setDisconnectedAt((previous) => previous ?? Date.now());
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
      if (ownsOperation())
        reportError(
          () => (cause &&
            typeof cause === "object" &&
            "name" in cause &&
            cause.name === "VoiceRequestTimeoutError"
            ? translateApp("Voice could not start. Try again.")
            : cause &&
                typeof cause === "object" &&
                "message" in cause &&
                typeof cause.message === "string"
              ? cause.message
              : translateApp("Voice could not start. Try again.")),
        );
    } finally {
      // A deadline abort still owns its UI until a newer action takes over.
      if (ownsOperation()) {
        busy.current = false;
        if (!current.current) setStarting(false);
      }
    }
    return true;
  }

  async function end(): Promise<boolean> {
    if (!sameChat()) return false;
    const previous = current.current;
    const cleanup = stopLocal();
    const generation = ticket.current;
    if (sameScope() && previous?.pendingEnd) {
      remember({
        session: previous.pendingEnd,
        thread,
        unknown: false,
        endedHere: true,
      });
      conflictChecks.current = 0;
    }
    await cleanup;
    if (sameScope() && ticket.current === generation && previous) {
      if (!previous.pendingStart)
        remember({
          session: previous.pendingEnd ?? null,
          endedHere: !!previous.pendingEnd,
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
      busy.current ||
      draining.current
    )
      return false;
    if (
      retainedRecovery?.epoch === environment.epoch &&
      retainedRecovery.pendingStart
    ) {
      if (manual)
        reportStatusError(
          () => (translateApp(
            "The previous voice start is still settling. Check again shortly.",
          )),
        );
      return false;
    }
    conflictBusy.current = true;
    const generation = ++ticket.current;
    const controller = new AbortController();
    pending.current = controller;
    const target = {
      ...latestConflict.current,
      endedHere:
        retainedRecovery?.session?.id === latestConflict.current.session?.id &&
        retainedRecovery?.endedHere,
    };
    const localSession = latestSnapshot.current?.session;
    // A completed client cleanup can clear recovery after local End has
    // fenced its final snapshot. Retain the owned snapshot's exact identity
    // until a read confirms terminal state or an empty active-call result.
    const priorTarget =
      !target.session &&
      localSession &&
      localSession.state !== "closed" &&
      localSession.state !== "failed"
        ? { ...target, session: localSession, thread, endedHere: true }
        : target;
    const owner = claimRecovery();
    remember(priorTarget, owner);
    const ownsOperation = () => sameScope() && ticket.current === generation;
    const valid = () => ownsOperation() && !controller.signal.aborted;
    setCheckingConflict(true);
    setConfirmedEnded(false);
    if (manual) {
      conflictChecks.current = 0;
      recoveryStarted.current = Date.now();
      reportError();
    }
    try {
      return await withVoiceRequestDeadline(controller, async () => {
        const discovered =
          priorTarget.endedHere &&
          typeof environment.api.accountActive === "function"
            ? await discoverActiveVoiceCall(
                environment.api,
                priorTarget.thread ?? thread,
                controller.signal,
              )
                .then((active) => ({ active }))
                .catch(() => undefined)
            : undefined;
        if (!valid()) return false;
        if (
          discovered?.active &&
          discovered.active.session.id !== priorTarget.session?.id
        ) {
          remember(
            {
              session: discovered.active.session,
              thread: discovered.active.conversationId,
              unknown: false,
            },
            owner,
          );
          reportStatusError();
          return false;
        }
        // An empty account slot must not bypass exact-session End delivery.
        if (priorTarget.session && priorTarget.thread) {
          try {
            let prior = voiceSessionSchema.parse(
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
              priorTarget.endedHere &&
              prior.state !== "closed" &&
              prior.state !== "failed"
            ) {
              if (!prior.end_requested) {
                const ended = voiceSessionSchema.parse(
                  await environment.api.control(
                    priorTarget.thread,
                    prior.id,
                    {
                      command_id: Crypto.randomUUID(),
                      expected_revision: prior.control_revision,
                      action: "end",
                    },
                    controller.signal,
                  ),
                );
                if (!valid() || ended.id !== prior.id) return false;
                prior = ended;
                remember(
                  {
                    session: ended,
                    thread: priorTarget.thread,
                    unknown: false,
                    endedHere: true,
                  },
                  owner,
                );
              } else {
                remember(
                  {
                    session: prior,
                    thread: priorTarget.thread,
                    unknown: false,
                    endedHere: true,
                  },
                  owner,
                );
              }
              if (prior.state !== "closed" && prior.state !== "failed") {
                if (discovered && !discovered.active && prior.end_requested) {
                  // The exact End is acknowledged and account admission proves
                  // the slot is empty; keep the transcript's last real snapshot.
                  remember({ session: null, unknown: false }, owner);
                  setConfirmedEnded(true);
                  reportStatusError();
                  return true;
                }
                return false;
              }
            }
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
              reportStatusError();
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
            endedHere:
              !!priorTarget.endedHere &&
              call?.session.id === priorTarget.session?.id,
          },
          owner,
        );
        setConfirmedEnded(!call);
        reportStatusError();
        return !call;
      });
    } catch {
      if (ownsOperation()) {
        remember({ ...priorTarget, unknown: true }, owner);
        reportStatusError(
          () => (translateApp(
            "Call status could not be verified. Check status again before starting voice.",
          )),
        );
      }
      return false;
    } finally {
      if (ownsOperation()) {
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
      {
        session: conflict,
        thread: conflictThread ?? thread,
        unknown: false,
        endedHere: true,
      },
      owner,
    );
    const controller = new AbortController();
    pending.current = controller;
    const ownsOperation = () => sameScope() && ticket.current === generation;
    const valid = () => ownsOperation() && !controller.signal.aborted;
    setEndingConflict(true);
    reportError();
    try {
      return await withVoiceRequestDeadline(controller, async () => {
        try {
          const next = voiceSessionSchema.parse(
            await environment.api.control(
              conflictThread ?? thread,
              conflict.id,
              {
                command_id: Crypto.randomUUID(),
                expected_revision: conflict.control_revision,
                action: "end",
              },
              controller.signal,
            ),
          );
          if (next.id !== conflict.id) throw localizedError("Voice session changed");
          if (valid()) {
            const terminal = next.state === "closed" || next.state === "failed";
            setConfirmedEnded(terminal);
            remember(
              {
                session: terminal ? null : next,
                endedHere: !terminal,
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
            () => (translateApp(
              "The existing call could not be ended. Refresh its state and try again.",
            )),
          );
          try {
            const active = await discoverActiveVoiceCall(
              environment.api,
              conflictThread ?? thread,
              controller.signal,
            );
            if (valid()) {
              setConfirmedEnded(!active);
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
        }
      });
    } catch {
      if (ownsOperation())
        reportError(
          () => (translateApp(
            "Ending the call could not be confirmed. Return to chat or check status again.",
          )),
        );
      return false;
    } finally {
      if (ownsOperation()) {
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
        reportError(
          () => (translateApp(
            "The task could not be stopped. Try again from the chat.",
          )),
        );
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
    connectedAt,
    disconnectedAt,
    inputEnabled,
    snapshot,
    holding,
    speakerMuted,
    error,
    recovery,
    microphonePermission,
    openSettings: () => {
      void Linking.openSettings().catch(() => {
        if (sameScope())
          reportError(
            () => (translateApp(
              Platform.OS === "ios"
                ? "Open Settings › NyxID Mobile › Microphone to allow microphone access."
                : "Open Settings › Apps › NyxID Mobile › Permissions › Microphone to allow microphone access.",
            )),
          );
      });
    },
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
        error === translateApp("Tap Resume audio to hear the assistant.")
      )
        reportError();
      return current.current?.resumeAudio();
    },
    stopTask,
  };
}
