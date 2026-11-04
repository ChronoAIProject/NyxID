import { useEffect, useRef, useState } from "react";
import { AppState, Linking } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import {
  ExpoSpeechRecognitionModule as Recognizer,
  type ExpoSpeechRecognitionErrorCode,
} from "expo-speech-recognition";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useWorkspaceAccessibility } from "../../lib/accessibility/workspaceAccessibility";
import { volumeLevel } from "./voice/dictationLevel";
import { dictationLocale } from "./voice/dictationLocale";
import { waitForVoiceForeground } from "./voice/voicePermission";

type Phase =
  | "idle"
  | "permission"
  | "connecting"
  | "listening"
  | "stopping"
  | "review";

const NO_SPEECH = "No speech was detected. Try again.";
const DENIED =
  "Allow microphone and speech recognition access in Settings to dictate. You can keep typing.";
/** A stopped recognizer that never reports `end` still releases the microphone. */
const STOP_TIMEOUT_MS = 8000;
/** How long a new attempt waits for an aborted one's `end`. */
const RELEASE_TIMEOUT_MS = 1500;

function dictationError(code: ExpoSpeechRecognitionErrorCode) {
  switch (code) {
    case "aborted":
      return undefined;
    case "not-allowed":
      return DENIED;
    case "no-speech":
    case "speech-timeout":
      return NO_SPEECH;
    case "language-not-supported":
      return "Dictation isn't available in your iPhone's language. You can keep typing.";
    case "network":
      return "Speech recognition needs a connection for your language. Check your connection and try again.";
    case "service-not-allowed":
      return "Speech recognition isn't available right now. Check that Dictation is on in Settings › General › Keyboard, then try again.";
    case "audio-capture":
      return "The microphone couldn't record. Close other apps using it and try again.";
    case "interrupted":
      return "Dictation was interrupted.";
    case "busy":
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
function release() {
  const done = new Promise<void>((resolve) => {
    const finish = () => {
      subscription.remove();
      clearTimeout(timer);
      if (releasing === done) releasing = undefined;
      resolve();
    };
    const subscription = Recognizer.addListener("end", finish);
    const timer = setTimeout(finish, RELEASE_TIMEOUT_MS);
  });
  releasing = done;
  Recognizer.abort();
}

async function recognitionLanguage() {
  let phone: string | undefined;
  try {
    phone = Intl.DateTimeFormat().resolvedOptions().locale;
  } catch {
    return undefined;
  }
  try {
    return dictationLocale(
      phone,
      (await Recognizer.getSupportedLocales({})).locales,
    );
  } catch {
    return phone;
  }
}

function supportsOnDevice() {
  try {
    return Recognizer.supportsOnDeviceRecognition();
  } catch {
    return false;
  }
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
 * Composer dictation on Apple's speech recognizer (on-device when the iPhone
 * supports it). Text stays in local review; nothing reaches NyxID until the
 * person inserts it and sends the message themselves.
 */
export function useComposerDictation({
  scope,
  enabled,
  onInsert,
}: {
  scope: string;
  enabled: boolean;
  onInsert: (text: string) => void;
}) {
  const focused = useIsFocused();
  const { sheetOpen } = useWorkspaceAccessibility();
  enabled = enabled && !sheetOpen;
  const epoch = getAuthEpoch();
  const [phase, setPhase] = useState<Phase>("idle");
  const [caption, setCaption] = useState({ final: "", interim: "" });
  const [error, setError] = useState<string>();
  const [permissionDenied, setPermissionDenied] = useState(false);
  const [levels] = useState(levelSource);
  const review = useRef<
    { scope: string; epoch: number; text: string } | undefined
  >(undefined);
  const current = useRef({ scope, epoch, enabled, focused, onInsert });
  current.current = { scope, epoch, enabled, focused, onInsert };
  const operation = useRef<{ cancel(): void; stop(): void } | undefined>(
    undefined,
  );

  function discard() {
    review.current = undefined;
    operation.current?.cancel();
    operation.current = undefined;
    setPhase("idle");
    setCaption({ final: "", interim: "" });
    setError(undefined);
    setPermissionDenied(false);
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
    const controller = new AbortController();
    const identity = { scope, epoch };
    let permission = true;
    /** Recognition was started and has not reported `end`: the mic may be on. */
    let native = false;
    let listening = false;
    let stopping = false;
    let finalText = "";
    let interimText = "";
    let failure: string | undefined;
    let subscriptions: { remove(): void }[] = [];
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
    const detach = () => {
      for (const subscription of subscriptions) subscription.remove();
      subscriptions = [];
      appState.remove();
      clearTimeout(stopTimer);
    };
    const cancel = () => {
      controller.abort();
      detach();
      if (native) {
        native = false;
        release();
      }
    };
    /** The recognizer ended: review what was heard, or explain why nothing was. */
    const settle = () => {
      detach();
      operation.current = undefined;
      const text = spoken();
      review.current = text ? { ...identity, text } : undefined;
      setCaption({ final: text, interim: "" });
      setPhase(text ? "review" : "idle");
      if (!text) setError((previous) => previous ?? failure ?? NO_SPEECH);
    };
    const attempt = {
      cancel,
      stop: () => {
        if (!valid() || stopping) return;
        if (!listening) {
          // Released before the microphone opened: nothing to review.
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
          release();
          settle();
        }, STOP_TIMEOUT_MS);
      },
    };
    const appState = AppState.addEventListener("change", (state) => {
      if (
        !valid() ||
        state === "active" ||
        (permission && state === "inactive")
      )
        return;
      discard();
    });
    operation.current = attempt;
    review.current = undefined;
    setCaption({ final: "", interim: "" });
    setPhase("permission");
    try {
      const result = await Recognizer.requestPermissionsAsync();
      if (!valid()) return;
      if (!result.granted) {
        setPermissionDenied(true);
        throw new Error(
          result.restricted
            ? "Speech recognition is restricted on this iPhone in Screen Time. You can keep typing."
            : DENIED,
        );
      }
      await waitForVoiceForeground(AppState, controller.signal, valid);
      permission = false;
      if (!valid()) return;
      setPhase("connecting");
      const lang = await recognitionLanguage();
      if (!valid()) return;
      await releasing;
      if (!valid()) return;
      const show = () =>
        setCaption({ final: finalText.trim(), interim: interimText.trim() });
      subscriptions = [
        Recognizer.addListener("start", () => {
          if (!valid()) return;
          listening = true;
          setPhase("listening");
        }),
        Recognizer.addListener("result", (event) => {
          if (!valid()) return;
          // Each final segment is appended (the recognizer supplies spacing);
          // the interim tail is replaced until it settles.
          const transcript = event.results[0]?.transcript ?? "";
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
          if (!valid()) return;
          const message = dictationError(event.error);
          if (!message || (message === NO_SPEECH && spoken())) return;
          if (event.error === "not-allowed") setPermissionDenied(true);
          failure = message;
          setError(message);
        }),
        Recognizer.addListener("end", () => {
          if (!valid()) return;
          native = false;
          settle();
        }),
      ];
      native = true;
      Recognizer.start({
        ...(lang ? { lang } : {}),
        interimResults: true,
        continuous: true,
        addsPunctuation: true,
        maxAlternatives: 1,
        iosTaskHint: "dictation",
        requiresOnDeviceRecognition: supportsOnDevice(),
        volumeChangeEventOptions: { enabled: true, intervalMillis: 90 },
      });
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
    error,
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
          "Open your iPhone Settings to allow microphone and speech recognition access.",
        ),
      );
    },
    insert: () => {
      const accepted = review.current;
      if (
        phase !== "review" ||
        !current.current.enabled ||
        !current.current.focused ||
        getAuthEpoch() !== epoch ||
        current.current.scope !== scope ||
        !accepted ||
        accepted.scope !== scope ||
        accepted.epoch !== epoch ||
        accepted.text !== text.trim()
      )
        return;
      review.current = undefined;
      current.current.onInsert(accepted.text);
      discard();
    },
  };
}
export type ComposerDictationState = ReturnType<typeof useComposerDictation>;
export function appendDictation(draft: string, text: string) {
  return `${draft}${draft && !/\s$/.test(draft) ? " " : ""}${text}`;
}
