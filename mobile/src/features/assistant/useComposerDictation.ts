import { useEffect, useMemo, useRef, useState } from "react";
import { AppState, Linking } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { Camera } from "expo-camera";
import * as Crypto from "expo-crypto";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useWorkspaceAccessibility } from "../../lib/accessibility/workspaceAccessibility";
import { nativeVoiceEnvironment } from "../../lib/api/assistantVoiceApi";
import {
  AssistantDictationClient,
  type DictationText,
} from "./voice/AssistantDictationClient";
import { NativeVoiceMedia } from "./voice/VoiceMediaBridge";
import { chooseVoiceOption } from "./voice/voiceSelection";
import { waitForVoiceForeground } from "./voice/voicePermission";

type Phase =
  | "idle"
  | "permission"
  | "connecting"
  | "listening"
  | "stopping"
  | "review";

export function useComposerDictation({
  scope,
  enabled,
  getThread,
  onInsert,
}: {
  scope: string;
  enabled: boolean;
  getThread: () => Promise<string>;
  onInsert: (text: string) => void;
}) {
  const focused = useIsFocused();
  const { sheetOpen } = useWorkspaceAccessibility();
  enabled = enabled && !sheetOpen;
  const epoch = getAuthEpoch();
  const setup = useMemo(() => {
    try {
      return { environment: nativeVoiceEnvironment() };
    } catch {
      return {
        error:
          "Dictation needs the configured NyxID server. You can keep typing.",
      };
    }
  }, [epoch]);
  const [phase, setPhase] = useState<Phase>("idle");
  const [caption, setCaption] = useState<DictationText>({
    final: "",
    interim: "",
  });
  const [error, setError] = useState<string>();
  const [permissionDenied, setPermissionDenied] = useState(false);
  const [media, setMedia] = useState<NativeVoiceMedia | null>(null);
  const review = useRef<
    { scope: string; epoch: number; text: string } | undefined
  >(undefined);
  const current = useRef({
    scope,
    epoch,
    enabled,
    focused,
    getThread,
    onInsert,
  });
  current.current = { scope, epoch, enabled, focused, getThread, onInsert };
  const operation = useRef<{ cancel(): void; stop(): void } | undefined>(
    undefined,
  );

  function discard() {
    review.current = undefined;
    operation.current?.cancel();
    operation.current = undefined;
    setMedia(null);
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
    if (!setup.environment) {
      setError(setup.error);
      return;
    }
    const environment = setup.environment;
    const controller = new AbortController();
    const identity = { scope, epoch };
    let permission = true;
    let client: AssistantDictationClient | undefined;
    let nextMedia: NativeVoiceMedia | undefined;
    let finalText = "";
    const valid = () =>
      operation.current === attempt &&
      !controller.signal.aborted &&
      current.current.scope === scope &&
      current.current.epoch === epoch &&
      getAuthEpoch() === epoch &&
      current.current.enabled &&
      current.current.focused;
    const cancel = () => {
      controller.abort();
      subscription.remove();
      client?.cancel();
      nextMedia?.close();
    };
    const attempt = {
      cancel,
      stop: () => {
        if (!valid()) return;
        if (client) client.stop();
        else {
          cancel();
          operation.current = undefined;
          setPhase("idle");
          setMedia(null);
        }
      },
    };
    const subscription = AppState.addEventListener("change", (state) => {
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
      const result = await Camera.requestMicrophonePermissionsAsync();
      if (!valid()) return;
      if (!result.granted) {
        setPermissionDenied(true);
        throw new Error(
          "Allow microphone access in Settings to dictate. You can keep typing.",
        );
      }
      await waitForVoiceForeground(AppState, controller.signal, valid);
      permission = false;
      if (!valid()) return;
      setPhase("connecting");
      // Same upload draft as web; permission denial never creates a conversation.
      const thread = await current.current.getThread();
      if (!valid()) return;
      const options = await environment.api.dictationOptions(
        thread,
        controller.signal,
      );
      if (!valid()) return;
      const option = chooseVoiceOption(options.options);
      if (!option)
        throw new Error(
          "Dictation is unavailable on this server. You can keep typing.",
        );
      nextMedia = new NativeVoiceMedia();
      client = new AssistantDictationClient({
        media: nextMedia,
        createSocket: environment.createSocket,
        randomId: Crypto.randomUUID,
        onText: (value) => {
          if (valid()) {
            finalText = value.final;
            setCaption(value);
          }
        },
        onError: (message) => {
          if (valid()) setError(message);
        },
        onPhase: (value) => {
          if (!valid()) return;
          if (value === "review") {
            subscription.remove();
            review.current = finalText.trim()
              ? { ...identity, text: finalText.trim() }
              : undefined;
            operation.current = undefined;
            setMedia(null);
            setCaption({ final: finalText, interim: "" });
            setPhase(finalText.trim() ? "review" : "idle");
            if (!finalText.trim())
              setError(
                (previous) => previous ?? "No speech was captured. Try again.",
              );
          } else setPhase(value);
        },
      });
      setMedia(nextMedia);
      await client.start(thread, {
        service_id: option.service_id,
        connection_id: null,
        key_source: "platform",
        model: option.model,
        voice: null,
        language: null,
        input_mode: "push_to_talk",
        notify_on_completion: false,
      });
    } catch (cause) {
      if (!valid()) return;
      cancel();
      operation.current = undefined;
      setMedia(null);
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
    error,
    permissionDenied,
    media,
    origin: setup.environment?.origin,
    active: phase !== "idle",
    recording: phase === "listening" || phase === "stopping",
    start,
    stop: () => operation.current?.stop(),
    discard,
    openSettings: () => {
      void Linking.openSettings().catch(() =>
        setError("Open your device Settings to allow microphone access."),
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
