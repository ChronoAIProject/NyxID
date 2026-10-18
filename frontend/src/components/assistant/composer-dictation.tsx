import {
  useCallback,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Check, ChevronDown, Hand, Square, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { DictationMicIcon } from "./dictation-mic-icon";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { api, ApiError } from "@/lib/api-client";
import {
  AssistantDictationClient,
  type DictationText,
} from "@/lib/assistant-dictation-client";
import {
  microphoneDevice,
  microphoneError,
  PreparedMicrophone,
  silentActivity,
} from "@/lib/microphone-capture";
import {
  voiceOptionsSchema,
  type VoiceOption,
  type VoicePreferences,
} from "@/schemas/assistant-voice";

function readMode(scope: string): "toggle" | "hold" {
  try {
    return localStorage.getItem(
      `nyxid:dictation-mode:${scope.split(":")[0]}`,
    ) === "hold"
      ? "hold"
      : "toggle";
  } catch {
    return "toggle";
  }
}
function quote(option?: VoiceOption) {
  return option
    ? `${option.model_label} · ${
        [option.pricing, ...(option.pricing?.components ?? [])]
          .filter(
            (p) =>
              p?.sync_status === "synced" &&
              ["voice_seconds", "requests"].includes(p.metric),
          )
          .map(
            (p) =>
              `${p!.credits_per_unit} credits per ${p!.metric === "requests" ? "start" : "input audio second"}`,
          )
          .join(" + ") || "Tariff unavailable"
      } · paid by you`
    : "Transcription rates are not available yet";
}
function discoveryMessage(error: unknown) {
  return error instanceof ApiError && error.status === 404
    ? "Dictation is unavailable on the connected server. You can still choose and test your microphone, or type your message."
    : "Dictation options could not be loaded. Check your connection and try again.";
}
export function useComposerDictation({
  scope,
  enabled,
  disabled,
  getThread,
  threadId,
  begin,
  insert,
  reviewClosed,
  configureVoice,
}: {
  scope: string;
  enabled: boolean;
  disabled: boolean;
  getThread?: () => Promise<string>;
  threadId?: string;
  begin: () => void;
  insert: (text: string) => boolean;
  reviewClosed?: () => void;
  configureVoice?: () => void;
}) {
  const account = scope.split(":")[0]!;
  const [open, setOpen] = useState(false);
  const hoverCloseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelHoverClose = useCallback(() => {
    if (hoverCloseTimer.current !== null) {
      clearTimeout(hoverCloseTimer.current);
      hoverCloseTimer.current = null;
    }
  }, []);
  const [mode, setMode] = useState<"toggle" | "hold">(() => readMode(scope));
  const [device, setDevice] = useState(() => microphoneDevice(account));
  const selectedDevice = useRef(device);
  selectedDevice.current = device;
  const deviceDiscovery = useRef(0);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [previewLevel, setPreviewLevel] = useState(0);
  const [, setPreviewRevision] = useState(0);
  const [stage, setStage] = useState<
    "idle" | "requesting" | "connecting" | "capturing" | "finalizing" | "review"
  >("idle");
  const [text, setText] = useState<DictationText>({ final: "", interim: "" });
  const [activity, setActivity] = useState(silentActivity);
  const [notice, setNotice] = useState("");
  const [optionsError, setOptionsError] = useState("");
  const [choices, setChoices] = useState<VoiceOption[]>([]);
  const [selection, setSelection] = useState("");
  const client = useRef<AssistantDictationClient | null>(null);
  const preview = useRef<PreparedMicrophone | null>(null);
  const generation = useRef(0);
  const busy = useRef(false);
  const held = useRef(false);
  const gesture = useRef<{ pointer?: number; key?: string } | null>(null);
  const [renderedScope, setRenderedScope] = useState(scope);
  if (renderedScope !== scope) {
    setRenderedScope(scope);
    setOpen(false);
    setMode(readMode(scope));
    const nextDevice = microphoneDevice(account);
    selectedDevice.current = nextDevice;
    setDevice(nextDevice);
    setDevices([]);
    setPreviewLevel(0);
    setStage("idle");
    setText({ final: "", interim: "" });
    setActivity(silentActivity());
    setNotice("");
    setOptionsError("");
    setChoices([]);
    setSelection("");
  }
  const option = selection
    ? choices.find((item) => item.service_id + item.model === selection)
    : (choices.find((item) => item.available && item.default_model) ??
      choices.find((item) => item.available));
  const active = stage !== "idle";
  const capturing = ["requesting", "connecting", "capturing"].includes(stage);
  const releasePreview = useCallback(() => {
    const old = preview.current;
    preview.current = null;
    old?.close();
  }, []);
  const invalidate = useCallback(() => {
    cancelHoverClose();
    generation.current++;
    client.current?.cancel();
    client.current = null;
    releasePreview();
  }, [releasePreview, cancelHoverClose]);
  useLayoutEffect(() => {
    busy.current = false;
    held.current = false;
    gesture.current = null;
    return invalidate;
  }, [scope, invalidate]);
  const refreshDevices = useCallback(async () => {
    const ticket = generation.current;
    const discovery = ++deviceDiscovery.current;
    try {
      const found = await navigator.mediaDevices?.enumerateDevices?.();
      if (
        generation.current === ticket &&
        deviceDiscovery.current === discovery &&
        found
      ) {
        const inputs = found.filter((item) => item.kind === "audioinput");
        setDevices(inputs);
        const currentDevice = selectedDevice.current;
        if (
          currentDevice &&
          inputs.some((item) => item.label) &&
          !inputs.some((item) => item.deviceId === currentDevice)
        ) {
          releasePreview();
          setPreviewLevel(0);
          setNotice(
            "Your selected microphone is disconnected. Choose an available microphone.",
          );
        }
      }
    } catch {
      if (
        generation.current === ticket &&
        deviceDiscovery.current === discovery
      )
        setNotice(
          "Microphone devices could not be listed. Try enabling your microphone.",
        );
    }
  }, [releasePreview]);
  useEffect(() => {
    if (!open) return;
    void refreshDevices();
    const update = () => {
      void refreshDevices();
    };
    navigator.mediaDevices?.addEventListener?.("devicechange", update);
    return () =>
      navigator.mediaDevices?.removeEventListener?.("devicechange", update);
  }, [open, scope, refreshDevices]);
  useEffect(() => {
    // Hover is read-only: never provision a fresh draft or request permission.
    if (!open || !threadId || !enabled || choices.length) return;
    const ticket = generation.current;
    let alive = true;
    void api
      .get(
        `/assistant/nyxagent/dictation/options?conversation_id=${encodeURIComponent(threadId)}`,
      )
      .then((data) => {
        if (alive && generation.current === ticket) {
          setChoices(voiceOptionsSchema.parse(data).options);
          setOptionsError("");
        }
      })
      .catch((error: unknown) => {
        if (alive && generation.current === ticket) {
          setOptionsError(discoveryMessage(error));
          setNotice(discoveryMessage(error));
        }
      });
    return () => {
      alive = false;
    };
  }, [open, threadId, enabled, choices.length]);
  function configure(next: boolean) {
    cancelHoverClose();
    setOpen(next);
    if (!next) {
      releasePreview();
      setPreviewLevel(0);
    }
  }
  function scheduleHoverClose() {
    cancelHoverClose();
    if (!open) return;
    // The content is portaled with an 8px gap. Allow crossing it in either
    // direction without dismissing the settings or their microphone preview.
    hoverCloseTimer.current = setTimeout(() => configure(false), 200);
  }
  function enableDevice(id = selectedDevice.current) {
    releasePreview();
    setPreviewLevel(0);
    setNotice("");
    try {
      const next = new PreparedMicrophone(id);
      preview.current = next;
      setPreviewRevision((n) => n + 1);
      next.listen(
        (value) => {
          if (preview.current === next) setPreviewLevel(value.input);
        },
        () => {
          if (preview.current === next) {
            releasePreview();
            setPreviewLevel(0);
            setNotice("Microphone access ended. Enable it to try again.");
          }
        },
      );
      void next.ready
        .then(() => {
          if (preview.current === next) void refreshDevices();
        })
        .catch((error: unknown) => {
          if (preview.current === next) {
            releasePreview();
            setPreviewLevel(0);
            setNotice(microphoneError(error));
          }
        });
    } catch (error) {
      setNotice(microphoneError(error));
    }
  }
  function selectDevice(id: string) {
    selectedDevice.current = id;
    deviceDiscovery.current++;
    setDevice(id);
    try {
      localStorage.setItem(`nyxid:microphone-device:${account}`, id);
    } catch {
      /* Optional local preference. */
    }
    enableDevice(id);
  }
  function start() {
    if (busy.current || disabled || !getThread) return;
    busy.current = true;
    configure(false);
    begin();
    setNotice("");
    setText({ final: "", interim: "" });
    const ticket = ++generation.current;
    let preparationNotice = "";
    let failed = false;
    let receivedText = false;
    const next = new AssistantDictationClient({
      text: (value) => {
        if (generation.current === ticket) {
          receivedText = Boolean(value.final.trim() || value.interim.trim());
          setText(value);
        }
      },
      activity: (value) => {
        if (generation.current === ticket) setActivity(value);
      },
      state: (value) => {
        if (generation.current === ticket) {
          if (value === "review" && failed && !receivedText) {
            busy.current = false;
            reviewClosed?.();
            setStage("idle");
            setActivity(silentActivity());
          } else setStage(value);
        }
      },
      error: (message) => {
        if (generation.current === ticket) {
          failed = true;
          setNotice(preparationNotice || message);
        }
      },
    });
    client.current = next;
    const discover = (id: string) =>
      api.get(
        `/assistant/nyxagent/dictation/options?conversation_id=${encodeURIComponent(id)}`,
      );
    let discovery: Promise<unknown> | undefined;
    // start constructs capture and encoding context synchronously in this gesture;
    // source preparation happens after permission, without forwarding any audio.
    void next.start(
      async () => {
        try {
          if (generation.current !== ticket)
            throw new DOMException("Cancelled", "AbortError");
          const id = await getThread();
          const data = voiceOptionsSchema.parse(
            await (id === threadId && discovery ? discovery : discover(id)),
          );
          if (generation.current !== ticket)
            throw new DOMException("Cancelled", "AbortError");
          const selected = selection
            ? data.options.find(
                (item) => item.service_id + item.model === selection,
              )
            : (data.options.find(
                (item) => item.available && item.default_model,
              ) ?? data.options.find((item) => item.available));
          setOptionsError("");
          setChoices(data.options);
          if (!selected?.available)
            throw new Error("Dictation source unavailable");
          const preferences: VoicePreferences = {
            service_id: selected.service_id,
            connection_id: null,
            key_source: "platform",
            model: selected.model,
            voice: null,
            language: null,
            input_mode: "push_to_talk",
            notify_on_completion: false,
          };
          return { thread: id, preferences };
        } catch (error) {
          preparationNotice = discoveryMessage(error);
          if (generation.current === ticket) setOptionsError(preparationNotice);
          throw error;
        }
      },
      undefined,
      device,
    );
    // Capture above requests permission synchronously. For an existing thread,
    // overlap read-only discovery with browser/device startup. Draft creation
    // and the metered socket still wait for microphone permission.
    if (threadId) {
      discovery = discover(threadId);
      // Denial/cancellation may prevent the preparation callback from awaiting it.
      void discovery.catch(() => {});
    }
  }
  function stop() {
    held.current = false;
    gesture.current = null;
    client.current?.stop();
  }
  function cancel(restoreFocus = true) {
    if (restoreFocus) reviewClosed?.();
    invalidate();
    busy.current = false;
    held.current = false;
    gesture.current = null;
    setStage("idle");
    setText({ final: "", interim: "" });
    setActivity(silentActivity());
    setNotice("");
  }
  useEffect(() => {
    const pointer = (event: PointerEvent) => {
      if (gesture.current?.pointer === event.pointerId) stop();
    };
    const key = (event: KeyboardEvent) => {
      if (gesture.current?.key === event.key) {
        event.preventDefault();
        stop();
      }
    };
    const blur = () => {
      if (held.current) stop();
      configure(false);
    };
    window.addEventListener("pointerup", pointer);
    window.addEventListener("pointercancel", pointer);
    window.addEventListener("keyup", key);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("pointerup", pointer);
      window.removeEventListener("pointercancel", pointer);
      window.removeEventListener("keyup", key);
      window.removeEventListener("blur", blur);
    };
  });
  useEffect(() => {
    if (!active && !open) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (active) cancel();
        configure(false);
      }
    };
    const pause = () => {
      if (document.visibilityState === "hidden") {
        stop();
        configure(false);
      }
    };
    window.addEventListener("keydown", escape);
    document.addEventListener("visibilitychange", pause);
    return () => {
      window.removeEventListener("keydown", escape);
      document.removeEventListener("visibilitychange", pause);
    };
  });
  const cancelIneligible = useEffectEvent(() => {
    cancel(false);
    setOpen(false);
    setPreviewLevel(0);
  });
  useEffect(() => {
    if (!enabled || disabled) cancelIneligible();
  }, [enabled, disabled]);
  const pressHandlers = {
    onPointerDown: (event: React.PointerEvent<HTMLButtonElement>) => {
      if (mode !== "hold" || event.button !== 0 || busy.current) return;
      event.preventDefault();
      held.current = true;
      gesture.current = { pointer: event.pointerId };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      start();
    },
    onPointerUp: () => {
      if (held.current) stop();
    },
    onPointerCancel: () => {
      if (held.current) stop();
    },
    onLostPointerCapture: () => {
      if (held.current) stop();
    },
    onKeyDown: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (mode === "hold" && [" ", "Enter"].includes(event.key)) {
        event.preventDefault();
        if (!event.repeat && !busy.current) {
          held.current = true;
          gesture.current = { key: event.key };
          start();
        }
      }
    },
    onKeyUp: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (mode === "hold" && [" ", "Enter"].includes(event.key)) {
        event.preventDefault();
        if (held.current) stop();
      }
    },
  };
  const button = enabled ? (
    <Popover open={open && !active} onOpenChange={configure}>
      <div
        className="dictation-split-control"
        onMouseEnter={cancelHoverClose}
        onMouseLeave={scheduleHoverClose}
      >
        <Button
          type="button"
          size="icon"
          variant="ghost"
          className={
            active
              ? "[&_svg]:size-5 text-primary hover:text-primary focus-visible:text-primary active:text-primary"
              : "[&_svg]:size-5"
          }
          aria-label={capturing ? "Stop dictation" : "Dictate a message"}
          aria-pressed={capturing}
          disabled={disabled || stage === "finalizing" || stage === "review"}
          {...pressHandlers}
          onClick={() => {
            if (mode === "hold") return;
            if (capturing) stop();
            else start();
          }}
        >
          <DictationMicIcon />
        </Button>
        <PopoverTrigger asChild>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label="Configure dictation"
            className="dictation-chevron"
            disabled={disabled || active}
            onMouseMove={(event) => {
              // Revealing this button can move it under a stationary pointer.
              // Open only when the pointer actually moves onto the chevron.
              if (!disabled && !active && (event.movementX || event.movementY))
                configure(true);
            }}
            onClick={(event) => {
              event.preventDefault();
              configure(true);
            }}
          >
            <ChevronDown className="size-3" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent
        onMouseEnter={cancelHoverClose}
        onMouseLeave={scheduleHoverClose}
        side="bottom"
        align="end"
        sideOffset={8}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onCloseAutoFocus={(event) => event.preventDefault()}
        className="w-72 max-w-[calc(100vw-2rem)] rounded-2xl border-hairline bg-popover p-3 text-13 shadow-lg"
      >
        <div
          className="mb-2 flex h-8 items-center gap-3 px-1"
          aria-label="Microphone input level"
          data-capture-active={Boolean(preview.current) || undefined}
        >
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label={
              preview.current ? "Stop microphone test" : "Test microphone"
            }
            title={preview.current ? "Stop microphone test" : "Test microphone"}
            onClick={() => {
              if (preview.current) {
                releasePreview();
                setPreviewLevel(0);
                setPreviewRevision((n) => n + 1);
              } else enableDevice();
            }}
          >
            <DictationMicIcon className="size-4 shrink-0 text-text-tertiary" />
          </Button>
          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-overlay">
            <div
              className="h-full origin-left rounded-full bg-primary transition-transform motion-reduce:transition-none"
              style={{ transform: `scaleX(${previewLevel})` }}
            />
          </div>
        </div>
        <div aria-label="Microphone devices" className="space-y-0.5">
          {devices.map((item, index) => (
            <button
              key={item.deviceId || index}
              type="button"
              className="flex min-h-10 w-full items-center gap-3 rounded-lg px-1.5 text-left hover:bg-overlay focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-pressed={device ? device === item.deviceId : index === 0}
              onClick={() => selectDevice(item.deviceId)}
            >
              <span className="min-w-0 flex-1 truncate">
                {item.label || `Microphone ${index + 1}`}
              </span>
              {(device ? device === item.deviceId : index === 0) && (
                <Check className="size-4 text-primary" />
              )}
            </button>
          ))}
        </div>
        {!devices.some((item) => item.label) && (
          <Button
            type="button"
            variant="ghost"
            className="w-full justify-start"
            disabled={disabled}
            onClick={() => enableDevice()}
          >
            Enable microphone
          </Button>
        )}
        <div className="mt-2 flex min-h-12 items-center gap-3 border-t border-hairline px-1 pt-2">
          <Hand className="size-4 text-text-secondary" />
          <label htmlFor={`hold-${scope}`} className="flex-1">
            Hold to record
          </label>
          <Switch
            id={`hold-${scope}`}
            aria-label="Hold to record"
            checked={mode === "hold"}
            onCheckedChange={(checked) => {
              const value = checked ? "hold" : "toggle";
              setMode(value);
              try {
                localStorage.setItem(`nyxid:dictation-mode:${account}`, value);
              } catch {
                /* Optional preference. */
              }
            }}
          />
        </div>
        <details className="mt-2 text-11 text-text-tertiary">
          <summary className="cursor-pointer">
            Transcription source
          </summary>
          <p className="mt-1">
            {option?.model_label ?? "Platform transcription"}. Review before
            inserting; never auto-sends. Maximum 2 minutes.
          </p>
          {choices.length > 1 && (
            <select
              aria-label="Dictation source"
              value={option ? option.service_id + option.model : ""}
              onChange={(event) => setSelection(event.target.value)}
              className="mt-1 w-full rounded border border-input bg-background p-1"
            >
              {choices.map((item) => (
                <option
                  key={item.service_id + item.model}
                  value={item.service_id + item.model}
                  disabled={!item.available}
                >
                  {item.model_label} · Platform
                </option>
              ))}
            </select>
          )}
        </details>
        <details className="mt-2 text-11 text-text-tertiary">
          <summary className="cursor-pointer">Credit usage</summary>
          <p className="mt-1">{optionsError || `${quote(option)}.`}</p>
          <p className="mt-1">
            View recorded charges in your account menu under Usage.
          </p>
        </details>
        {configureVoice && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="mt-1 w-full justify-start text-text-secondary"
            onClick={configureVoice}
          >
            Voice settings…
          </Button>
        )}
        {notice && (
          <p role="alert" className="mt-2 text-12 text-destructive">
            {notice}
          </p>
        )}
      </PopoverContent>
    </Popover>
  ) : null;
  const review = active ? (
    <div className="space-y-3 py-2" aria-label="Dictation review">
      <div className="min-h-10 whitespace-pre-wrap text-13">
        {text.parts?.length ? (
          text.parts.map((part, index) => (
            <span
              key={part.order}
              className={
                part.complete ? undefined : "italic text-text-tertiary"
              }
            >
              {index ? " " : ""}
              {part.text}
            </span>
          ))
        ) : (
          <>
            <span>{text.final}</span>
            {text.interim && (
              <span className="italic text-text-tertiary">
                {text.final ? " " : ""}
                {text.interim}
              </span>
            )}
          </>
        )}
        {!text.final && !text.interim && (
          <span className="text-text-tertiary">
            {stage === "requesting"
              ? "Allow microphone access in your browser…"
              : stage === "connecting"
                ? "Connecting dictation…"
                : stage === "review"
                  ? "No speech captured."
                  : "Speak your message…"}
          </span>
        )}
      </div>
      <div className="flex items-center gap-3">
        <div
          aria-hidden
          className="dictation-waveform flex h-8 min-w-0 flex-1 items-center gap-1"
        >
          {activity.bands.map((level, index) => (
            <span
              key={index}
              className="min-w-0 flex-1 rounded-full bg-primary"
              style={{ height: `${Math.max(2, level * 30)}px` }}
            />
          ))}
        </div>
        <span role="status" className="text-11 text-text-tertiary">
          {stage === "capturing"
            ? "Listening"
            : stage === "finalizing"
              ? "Finishing…"
              : stage === "review"
                ? "Review"
                : "Starting…"}
        </span>
        {capturing && (
          <Button
            type="button"
            size="icon"
            variant="outline"
            aria-label="Stop dictation for review"
            onClick={stop}
          >
            <Square className="size-3 fill-current" />
          </Button>
        )}
        <Button
          type="button"
          size="icon"
          variant="outline"
          aria-label="Cancel dictation"
          onClick={() => cancel()}
        >
          <X />
        </Button>
        <Button
          type="button"
          size="icon"
          variant="primary"
          aria-label="Insert dictation"
          disabled={stage !== "review" || !text.final.trim()}
          onClick={() => {
            if (insert(text.final.trim())) cancel();
            else
              setNotice(
                "Your draft changed. Cancel dictation and try again to preserve its contents.",
              );
          }}
        >
          <Check />
        </Button>
      </div>
      {notice && (
        <p role="alert" className="text-12 text-destructive">
          {notice}
        </p>
      )}
    </div>
  ) : notice && !open ? (
    <p role="alert" className="pb-2 text-12 text-destructive">
      {notice}
    </p>
  ) : null;
  return {
    button,
    review,
    active,
    captureActive: capturing,
    activity,
    cancel,
    releaseMicrophone: () => configure(false),
  };
}
