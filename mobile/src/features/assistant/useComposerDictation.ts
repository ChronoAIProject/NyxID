import { useEffect, useRef, useState } from "react";
import { AppState, Linking, Platform } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import {
  ExpoSpeechRecognitionModule as Recognizer,
  type ExpoSpeechRecognitionErrorCode,
} from "expo-speech-recognition";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useWorkspaceAccessibility } from "../../lib/accessibility/workspaceAccessibility";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { deviceLanguages } from "../../i18n/deviceLanguages";
import { languageLabel } from "./voice/dictationLanguage";
import { volumeLevel } from "./voice/dictationLevel";
import { recognitionLocales } from "./voice/recognitionLocales";
import { resolveDictationLanguage } from "./voice/resolveDictationLanguage";
import { savedDictationLanguage } from "./voice/dictationLanguage";
import { waitForVoiceForeground } from "./voice/voicePermission";
import { claimVoiceAudioSession } from "./voice/audioSessionOwnership";
import { claimSystemStatusActivity } from "../../lib/systemStatusActivity";

type Phase = "idle" | "permission" | "connecting" | "listening" | "stopping";

const NO_SPEECH = "No speech was detected. Try again.";
const DENIED =
  "Allow microphone and speech recognition access in Settings to dictate. You can keep typing.";
const START_FAILED =
  "Dictation could not start. Please try again. You can keep typing.";
/** Give people time to read the native prompt, separately from the foreground handoff. */
const PERMISSION_TIMEOUT_MS = 60_000;
/** Locale discovery or a native recognizer must not leave the composer starting forever. */
const START_TIMEOUT_MS = 8000;
/** A stopped recognizer that never reports `end` still releases the microphone. */
const STOP_TIMEOUT_MS = 8000;
/** How long a new attempt waits for an aborted one's `end`. */
const RELEASE_TIMEOUT_MS = 1500;
/** Some Android services ignore silence intent extras and stop after ~4s.
 * Restart only before any speech, inside one bounded user recording. */
const ANDROID_INITIAL_SILENCE_MS = 15_000;
const MAX_SILENCE_RESTARTS = 3;

function permissionDeniedMessage() {
  return Platform.OS === "android"
    ? "Allow microphone access in Settings to dictate. You can keep typing."
    : DENIED;
}

/** Both prompt denial and native errors use the actual iOS permission pair. */
async function deniedPermissionMessage(permissions: {
  status: "granted" | "denied" | "undetermined";
  granted: boolean;
  restricted?: boolean;
}): Promise<string | undefined> {
  if (permissions.granted && !permissions.restricted) return undefined;
  const restricted = () =>
    Platform.OS === "ios"
      ? "Speech recognition is restricted on this device in Screen Time. You can keep typing."
      : "Speech recognition is restricted on this device. You can keep typing.";
  if (permissions.restricted) return restricted();
  if (Platform.OS !== "ios") return permissionDeniedMessage();
  if (permissions.status !== "denied") return undefined;
  try {
    const [mic, speech] = await Promise.all([
      Recognizer.getMicrophonePermissionsAsync(),
      Recognizer.getSpeechRecognizerPermissionsAsync(),
    ]);
    if (speech.restricted) return restricted();
    if (mic.status === "denied" && speech.status === "denied") return DENIED;
    if (mic.status === "denied")
      return "Allow microphone access in Settings to dictate. You can keep typing.";
    if (speech.status === "denied")
      return "Allow speech recognition access in Settings to dictate. You can keep typing.";
    return undefined;
  } catch {
    // The combined response already establishes denial if individual reads fail.
  }
  return permissionDeniedMessage();
}

function dictationError(
  code: ExpoSpeechRecognitionErrorCode | "too-many-requests",
) {
  switch (code) {
    case "aborted":
      return undefined;
    case "not-allowed":
      return permissionDeniedMessage();
    case "no-speech":
    case "speech-timeout":
      return NO_SPEECH;
    case "language-not-supported":
      return Platform.OS === "android"
        ? "Dictation isn't available in your selected language. Choose another language or keep typing."
        : "Dictation isn't available in your device's language. You can keep typing.";
    case "network":
      return "Speech recognition couldn't continue. Check your connection and try again.";
    case "service-not-allowed":
      return Platform.OS === "android"
        ? "Speech recognition is temporarily unavailable. Try again. If this continues, check your device's speech recognition service."
        : "Speech recognition is temporarily unavailable. Try again. If this continues, check Siri & Dictation in Settings.";
    case "audio-capture":
      return "The microphone couldn't record. Close other apps using it and try again.";
    case "interrupted":
      return "Dictation was interrupted.";
    case "busy":
    case "too-many-requests":
      return "Speech recognition is busy. Try again in a moment.";
    default:
      return "Dictation stopped unexpectedly. You can keep typing.";
  }
}

/**
 * The recognizer is one shared native session whose abort reports `end`
 * asynchronously. A new attempt waits for that, so a stale `end` can never
 * close it.
 */
let releasing: Promise<void> | undefined;
type AudioOwner = ReturnType<typeof claimVoiceAudioSession>;
function deactivate(owner: AudioOwner) {
  owner.release(() => {
    if (Platform.OS === "ios") {
      try {
        Recognizer.setAudioSessionActiveIOS(false, {
          notifyOthersOnDeactivation: true,
        });
      } catch {
        // The native session may already have been released by the OS.
      }
    }
  });
}
function release(owner: AudioOwner, endCapture: () => void) {
  const done = new Promise<void>((resolve) => {
    const finish = () => {
      subscription.remove();
      clearTimeout(timer);
      deactivate(owner);
      endCapture();
      if (releasing === done) releasing = undefined;
      resolve();
    };
    const subscription = Recognizer.addListener("end", finish);
    const timer = setTimeout(finish, RELEASE_TIMEOUT_MS);
  });
  releasing = done;
  Recognizer.abort();
}

/** The chosen dictation language when still supported, else the device's. */
async function recognitionLanguage() {
  const chosen = await savedDictationLanguage();
  const preferred = deviceLanguages();
  const discovery = await recognitionLocales(Recognizer, Platform.OS);
  const locales = discovery?.locales ?? [];
  const installedLocales = discovery?.installedLocales ?? [];
  const lang = resolveDictationLanguage(
    chosen,
    preferred,
    locales,
    Platform.OS,
  );
  return {
    lang,
    service: discovery.service,
    canChooseLanguage: locales.length > 0 || Platform.OS === "android",
    unsupported:
      locales.length > 0 && !lang && (Platform.OS !== "android" || !!chosen),
    selected: chosen ?? lang ?? preferred[0],
    hasSavedChoice: !!chosen,
    // The iOS JS capability APIs only inspect the default locale. Native code
    // applies this request only if the selected recognizer supports it.
    onDevice:
      Platform.OS === "ios" ||
      (!!lang && (installedLocales ?? []).includes(lang)),
  };
}

function levelSource() {
  const listeners = new Set<(level: number) => void>();
  return {
    emit: (level: number) => listeners.forEach((listener) => listener(level)),
    /** Live microphone level, 0..1, about every 90ms while listening. */
    subscribe: (listener: (level: number) => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/**
 * Composer dictation on the device's speech recognizer (on-device when
 * supported). The words appear in the composer as they are recognized and
 * join the draft when recording finishes. Crew voice instead opts into native
 * utterance boundaries and sends successful final results through its sender.
 */
export function useComposerDictation({
  scope,
  enabled,
  onInsert,
  onSilence,
  continuous = true,
}: {
  scope: string;
  enabled: boolean;
  onInsert: (text: string) => void;
  /** Automatic utterance users can pace retries using native session duration. */
  onSilence?: (durationMs: number) => void;
  /** Crew voice ends each utterance at native silence detection. */
  continuous?: boolean;
}) {
  const { t, language } = useAppLanguage();
  const focused = useIsFocused();
  const { sheetOpen } = useWorkspaceAccessibility();
  enabled = enabled && !sheetOpen;
  const epoch = getAuthEpoch();
  const [phase, setPhase] = useState<Phase>("idle");
  const [caption, setCaption] = useState({ final: "", interim: "" });
  const [error, setError] = useState<string>();
  const [permissionDenied, setPermissionDenied] = useState(false);
  const [chooseLanguage, setChooseLanguage] = useState(false);
  const [recognitionLocale, setRecognitionLocale] = useState<string>();
  const [levels] = useState(levelSource);
  const current = useRef({
    scope,
    epoch,
    enabled,
    focused,
    onInsert,
    onSilence,
  });
  current.current = { scope, epoch, enabled, focused, onInsert, onSilence };
  const operation = useRef<{ cancel(): void; stop(): void } | undefined>(
    undefined,
  );

  function discard() {
    operation.current?.cancel();
    operation.current = undefined;
    setPhase("idle");
    setCaption({ final: "", interim: "" });
    setError(undefined);
    setPermissionDenied(false);
    setChooseLanguage(false);
  }
  useEffect(() => {
    discard();
    return () => {
      operation.current?.cancel();
      operation.current = undefined;
    };
  }, [scope, epoch, enabled, focused]);

  async function start() {
    if (
      operation.current ||
      !current.current.enabled ||
      !current.current.focused ||
      getAuthEpoch() !== epoch ||
      current.current.scope !== scope ||
      AppState.currentState !== "active"
    )
      return;
    setError(undefined);
    setPermissionDenied(false);
    setChooseLanguage(false);
    const controller = new AbortController();
    const identity = { scope, epoch };
    let permissionPrompt:
      | { androidBackgroundAllowed: boolean; androidBackgrounded: boolean }
      | undefined;
    /** Recognition was started and has not reported `end`: the mic may be on. */
    let native = false;
    let audioOwner: AudioOwner | undefined;
    // One utterance keeps the sender/routing that was visible at capture entry.
    const insert = current.current.onInsert;
    let listening = false;
    let stopping = false;
    let finalText = "";
    let interimText = "";
    let failure: string | undefined;
    let nativeStartedAt = 0;
    let lastNativeStartedAt = 0;
    let receivedResult = false;
    let heardSpeech = false;
    let silenceRestarts = 0;
    let serverRetried = false;
    let recovering: Promise<boolean> | undefined;
    let restartSilent: (() => void) | undefined;
    let retryWithServer: (() => void) | undefined;
    let subscriptions: { remove(): void }[] = [];
    let releaseStatus: (() => void) | undefined;
    const endCapture = () => {
      releaseStatus?.();
      releaseStatus = undefined;
    };
    let permissionTimer: ReturnType<typeof setTimeout> | undefined;
    let startTimer: ReturnType<typeof setTimeout> | undefined;
    let nativeEndTimer: ReturnType<typeof setTimeout> | undefined;
    let silenceTimer: ReturnType<typeof setTimeout> | undefined;
    let stopTimer: ReturnType<typeof setTimeout> | undefined;
    const spoken = () =>
      [finalText.trim(), interimText.trim()].filter(Boolean).join(" ");
    const valid = () =>
      operation.current === attempt &&
      !controller.signal.aborted &&
      current.current.scope === scope &&
      current.current.epoch === epoch &&
      getAuthEpoch() === epoch &&
      current.current.enabled &&
      current.current.focused;
    const canRestartSilence = () =>
      Platform.OS === "android" &&
      continuous &&
      !stopping &&
      !heardSpeech &&
      !spoken() &&
      silenceRestarts < MAX_SILENCE_RESTARTS &&
      Date.now() - nativeStartedAt < ANDROID_INITIAL_SILENCE_MS;
    const detach = () => {
      for (const subscription of subscriptions) subscription.remove();
      subscriptions = [];
      appState.remove();
      clearTimeout(permissionTimer);
      clearTimeout(startTimer);
      clearTimeout(stopTimer);
      clearTimeout(silenceTimer);
      clearTimeout(nativeEndTimer);
    };
    const cancel = () => {
      controller.abort();
      detach();
      if (native) {
        native = false;
        release(audioOwner!, endCapture);
      } else if (audioOwner) {
        deactivate(audioOwner);
        endCapture();
      }
    };
    /**
     * The recognizer ended: what was heard joins the draft, where it can be
     * edited before the person sends it, or the notice explains why nothing was.
     */
    const settle = () => {
      detach();
      endCapture();
      operation.current = undefined;
      const text = spoken();
      setCaption({ final: "", interim: "" });
      setPhase("idle");
      // Draft dictation can preserve an interrupted tail for review. Crew
      // speech dispatches automatically, so only a successful native final
      // result may cross its sender boundary.
      if (!continuous && (failure || interimText.trim() || !finalText.trim())) {
        const silence = !text && (!failure || failure === NO_SPEECH);
        setError(
          silence
            ? undefined
            : (failure ?? "Speech wasn't finalized. Please try again."),
        );
        if (silence) current.current.onSilence?.(Date.now() - nativeStartedAt);
        return;
      }
      if (!text) {
        setError((previous) => previous ?? failure ?? NO_SPEECH);
        return;
      }
      if (
        current.current.scope === identity.scope &&
        current.current.epoch === identity.epoch &&
        getAuthEpoch() === identity.epoch &&
        current.current.enabled &&
        current.current.focused
      )
        (continuous ? current.current.onInsert : insert)(text);
    };
    const attempt = {
      cancel,
      stop: () => {
        if (!valid() || stopping) return;
        if (!listening) {
          // Released before the microphone opened: nothing was heard.
          cancel();
          operation.current = undefined;
          setPhase("idle");
          return;
        }
        stopping = true;
        setPhase("stopping");
        Recognizer.stop();
        stopTimer = setTimeout(() => {
          if (!valid()) return;
          native = false;
          release(audioOwner!, endCapture);
          settle();
        }, STOP_TIMEOUT_MS);
      },
    };
    const appState = AppState.addEventListener("change", (state) => {
      if (!valid()) return;
      if (state === "active") {
        if (permissionPrompt?.androidBackgrounded) {
          permissionPrompt.androidBackgroundAllowed = false;
          permissionPrompt.androidBackgrounded = false;
        }
        return;
      }
      // Only a real native permission request may pause before capture starts.
      if (permissionPrompt && !native) {
        if (state === "inactive") return;
        if (
          state === "background" &&
          permissionPrompt.androidBackgroundAllowed
        ) {
          permissionPrompt.androidBackgrounded = true;
          return;
        }
      }
      discard();
    });
    operation.current = attempt;
    setCaption({ final: "", interim: "" });
    setPhase("permission");
    try {
      const available =
        Platform.OS !== "android" || Recognizer.isRecognitionAvailable();
      let result = await Recognizer.getPermissionsAsync();
      if (!valid()) return;
      if (AppState.currentState !== "active") throw new Error(START_FAILED);
      if (!available) {
        if (result.status === "denied" || result.restricted) {
          setPermissionDenied(true);
          throw new Error(permissionDeniedMessage());
        }
        throw new Error(
          "This device doesn't have a speech recognition service available. You can keep typing.",
        );
      }
      if (!result.granted) {
        permissionPrompt = {
          androidBackgroundAllowed: Platform.OS === "android",
          androidBackgrounded: false,
        };
        try {
          permissionTimer = setTimeout(() => {
            if (!valid()) return;
            cancel();
            operation.current = undefined;
            setPhase("idle");
            setError(START_FAILED);
          }, PERMISSION_TIMEOUT_MS);
          result = await Recognizer.requestPermissionsAsync();
          clearTimeout(permissionTimer);
          permissionTimer = undefined;
          if (!valid()) return;
          if (!result.granted) {
            const denied = await deniedPermissionMessage(result);
            if (!valid()) return;
            setPermissionDenied(!!denied);
            throw new Error(denied ?? START_FAILED);
          }
          try {
            await waitForVoiceForeground(AppState, controller.signal, valid, {
              allowInitialBackground: permissionPrompt.androidBackgroundAllowed,
            });
          } catch {
            if (!valid()) return;
            throw new Error(START_FAILED);
          }
        } finally {
          clearTimeout(permissionTimer);
          permissionPrompt = undefined;
        }
      }
      if (!valid()) return;
      if (AppState.currentState !== "active") throw new Error(START_FAILED);
      setPhase("connecting");
      startTimer = setTimeout(() => {
        if (!valid()) return;
        cancel();
        operation.current = undefined;
        setPhase("idle");
        setError((previous) => previous ?? START_FAILED);
      }, START_TIMEOUT_MS);
      const recognition = await recognitionLanguage();
      const { lang } = recognition;
      if (!valid()) return;
      setRecognitionLocale(
        lang ?? (recognition.unsupported ? recognition.selected : undefined),
      );
      if (recognition.unsupported) {
        setChooseLanguage(true);
        throw new Error(
          recognition.hasSavedChoice
            ? "Dictation isn't available in your selected language. Choose another language or keep typing."
            : "Dictation isn't available in your device's language. Choose a language or keep typing.",
        );
      }
      await releasing;
      if (!valid()) return;
      const show = () =>
        setCaption({ final: finalText.trim(), interim: interimText.trim() });
      subscriptions = [
        Recognizer.addListener("audiostart", () => {
          if (valid()) releaseStatus ??= claimSystemStatusActivity();
        }),
        Recognizer.addListener("audioend", endCapture),
        Recognizer.addListener("start", () => {
          if (!valid()) return;
          clearTimeout(startTimer);
          listening = true;
          setPhase("listening");
        }),
        Recognizer.addListener("speechstart", () => {
          if (valid()) heardSpeech = true;
        }),
        Recognizer.addListener("result", (event) => {
          if (!valid()) return;
          // Each final segment is appended (the recognizer supplies spacing);
          // the interim tail is replaced until it settles.
          receivedResult = true;
          const transcript = event.results[0]?.transcript ?? "";
          heardSpeech ||= !!transcript.trim();
          if (event.isFinal) {
            finalText += transcript;
            interimText = "";
          } else interimText = transcript;
          show();
        }),
        Recognizer.addListener("volumechange", (event) => {
          if (valid()) levels.emit(volumeLevel(event.value));
        }),
        Recognizer.addListener("nomatch", () => {
          if (valid()) failure ??= NO_SPEECH;
        }),
        Recognizer.addListener("error", (event) => {
          if (!valid() || !native || recovering) return;
          if (
            (event.error === "no-speech" || event.error === "speech-timeout") &&
            canRestartSilence()
          ) {
            failure = NO_SPEECH;
            clearTimeout(nativeEndTimer);
            nativeEndTimer = setTimeout(() => {
              if (!valid() || !native) return;
              native = false;
              release(audioOwner!, endCapture);
              failure = START_FAILED;
              settle();
            }, RELEASE_TIMEOUT_MS);
            return;
          }
          if (!["service-not-allowed", "not-allowed"].includes(event.error)) {
            const message =
              event.error === "language-not-supported" &&
              !recognition.canChooseLanguage
                ? START_FAILED
                : dictationError(event.error);
            if (!message || (message === NO_SPEECH && spoken())) return;
            if (
              event.error === "language-not-supported" &&
              recognition.canChooseLanguage
            )
              setChooseLanguage(true);
            failure = message;
            if (continuous || message !== NO_SPEECH) setError(message);
            return;
          }
          const classify = async () => {
            if (["service-not-allowed", "not-allowed"].includes(event.error)) {
              try {
                const permissions = await Recognizer.getPermissionsAsync();
                if (!valid()) return false;
                const denied = await deniedPermissionMessage(permissions);
                if (!valid()) return false;
                if (denied) {
                  setPermissionDenied(true);
                  failure = denied;
                  setError(denied);
                  return false;
                }
              } catch {
                // A failed permission read is not evidence of denial.
              }
              if (
                Platform.OS === "ios" &&
                event.error === "service-not-allowed" &&
                recognition.onDevice &&
                !serverRetried &&
                !receivedResult &&
                !stopping
              ) {
                serverRetried = true;
                return true;
              }
            }
            if (!valid()) return false;
            const message =
              event.error === "language-not-supported" &&
              !recognition.canChooseLanguage
                ? START_FAILED
                : dictationError(event.error);
            if (!message || (message === NO_SPEECH && spoken())) return false;
            if (
              event.error === "language-not-supported" &&
              recognition.canChooseLanguage
            )
              setChooseLanguage(true);
            if (event.error === "not-allowed") setPermissionDenied(true);
            failure = message;
            if (continuous || message !== NO_SPEECH) setError(message);
            return false;
          };
          // Permission reads can finish after native end. The end handler waits
          // before deciding whether to settle or retry this same recording.
          recovering = classify();
        }),
        Recognizer.addListener("end", () => {
          if (!valid() || !native) return;
          native = false;
          clearTimeout(nativeEndTimer);
          if (
            !recovering &&
            (!failure || failure === NO_SPEECH) &&
            canRestartSilence()
          ) {
            silenceRestarts++;
            failure = undefined;
            listening = false;
            // Native end is the release boundary. Back off briefly, keeping
            // this attempt cancellable and its locale/draft ownership intact.
            silenceTimer = setTimeout(
              () => {
                if (!valid()) return;
                if (
                  Date.now() - nativeStartedAt >=
                  ANDROID_INITIAL_SILENCE_MS
                ) {
                  failure = NO_SPEECH;
                  deactivate(audioOwner!);
                  settle();
                } else restartSilent?.();
              },
              Math.max(250, 5000 - (Date.now() - lastNativeStartedAt)),
            );
            return;
          }
          const finish = (retry: boolean) => {
            recovering = undefined;
            if (!valid()) return;
            if (retry && !stopping && !receivedResult) {
              retryWithServer?.();
              return;
            }
            deactivate(audioOwner!);
            settle();
          };
          if (recovering) void recovering.then(finish);
          else finish(false);
        }),
      ];
      native = true;
      audioOwner = claimVoiceAudioSession();
      nativeStartedAt = Date.now();
      const beginNative = (onDevice: boolean, silentRestart = false) => {
        native = true;
        listening = false;
        if (!silentRestart) setPhase("connecting");
        clearTimeout(startTimer);
        startTimer = setTimeout(() => {
          if (!valid()) return;
          cancel();
          operation.current = undefined;
          setPhase("idle");
          setError(START_FAILED);
        }, START_TIMEOUT_MS);
        lastNativeStartedAt = Date.now();
        Recognizer.start({
          lang,
          ...(Platform.OS === "android"
            ? {
                androidRecognitionServicePackage: recognition.service,
                androidIntentOptions: {
                  EXTRA_PREFER_OFFLINE: onDevice,
                  ...(continuous
                    ? {
                        EXTRA_SPEECH_INPUT_MINIMUM_LENGTH_MILLIS:
                          ANDROID_INITIAL_SILENCE_MS,
                        EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS: 3000,
                        EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS: 3000,
                      }
                    : {}),
                },
              }
            : {}),
          interimResults: true,
          continuous,
          addsPunctuation: true,
          maxAlternatives: 1,
          iosTaskHint: "dictation",
          // expo's Android requiresOnDevice flag selects a separate system
          // service ahead of the explicit package. Keep discovery/start on the
          // chosen provider, asking it to prefer offline instead.
          requiresOnDeviceRecognition:
            Platform.OS === "android" && recognition.service ? false : onDevice,
          volumeChangeEventOptions: { enabled: true, intervalMillis: 90 },
        });
      };
      restartSilent = () => {
        try {
          beginNative(recognition.onDevice, true);
        } catch {
          failure = START_FAILED;
          deactivate(audioOwner!);
          settle();
        }
      };
      retryWithServer = () => {
        try {
          beginNative(false);
        } catch {
          failure = START_FAILED;
          deactivate(audioOwner!);
          settle();
        }
      };
      beginNative(recognition.onDevice);
    } catch (cause) {
      if (!valid()) return;
      cancel();
      operation.current = undefined;
      setPhase("idle");
      setError(
        cause instanceof Error
          ? cause.message
          : "Dictation could not start. You can keep typing.",
      );
    }
  }
  const text = [caption.final, caption.interim].filter(Boolean).join(" ");
  return {
    phase,
    text,
    /** Settled words and the recognizer's still-changing tail, for display only. */
    final: caption.final,
    interim: caption.interim,
    error: error
      ? chooseLanguage &&
        error !==
          "Dictation isn't available in your device's language. Choose a language or keep typing."
        ? t(
            "Dictation isn't available in {{language}}. Choose another language or keep typing.",
            {
              language: recognitionLocale
                ? languageLabel(recognitionLocale, language)
                : t("Device language"),
            },
          )
        : t(error)
      : undefined,
    recognitionLocale,
    chooseLanguage,
    retry: start,
    permissionDenied,
    subscribeLevel: levels.subscribe,
    active: phase !== "idle",
    recording: phase === "listening" || phase === "stopping",
    start,
    stop: () => operation.current?.stop(),
    discard,
    openSettings: () => {
      void Linking.openSettings().catch(() =>
        setError(
          Platform.OS === "android"
            ? "Open your device Settings to allow microphone access."
            : "Open your device Settings to allow microphone and speech recognition access.",
        ),
      );
    },
  };
}
export type ComposerDictationState = ReturnType<typeof useComposerDictation>;
export function appendDictation(draft: string, text: string) {
  return `${draft}${draft && !/\s$/.test(draft) ? " " : ""}${text}`;
}
