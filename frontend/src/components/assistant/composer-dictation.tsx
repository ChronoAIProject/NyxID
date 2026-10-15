import {
  useCallback,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Check, ChevronDown, Mic, Square, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { api } from "@/lib/api-client";
import {
  AssistantDictationClient,
  type DictationText,
} from "@/lib/assistant-dictation-client";
import { silentActivity } from "@/lib/microphone-capture";
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

export function useComposerDictation({
  scope,
  enabled,
  disabled,
  getThread,
  begin,
  insert,
  reviewClosed,
}: {
  scope: string;
  enabled: boolean;
  disabled: boolean;
  getThread?: () => Promise<string>;
  begin: () => void;
  insert: (text: string) => boolean;
  reviewClosed?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"toggle" | "hold">(() => readMode(scope));
  const [stage, setStage] = useState<
    "idle" | "requesting" | "connecting" | "capturing" | "finalizing" | "review"
  >("idle");
  const [text, setText] = useState<DictationText>({ final: "", interim: "" });
  const [activity, setActivity] = useState(silentActivity);
  const [notice, setNotice] = useState("");
  const [choices, setChoices] = useState<VoiceOption[]>([]);
  const [selection, setSelection] = useState("");
  const [thread, setThread] = useState("");
  const [loading, setLoading] = useState(false);
  const client = useRef<AssistantDictationClient | null>(null);
  const generation = useRef(0);
  const busy = useRef(false);
  const [approved, setApproved] = useState(false);
  const [holding, setHolding] = useState(false);
  const held = useRef(false);
  const gesture = useRef<{ pointer?: number; key?: string } | null>(null);
  const [renderedScope, setRenderedScope] = useState(scope);
  if (renderedScope !== scope) {
    setRenderedScope(scope);
    setApproved(false);
    setHolding(false);
    setOpen(false);
    setMode(readMode(scope));
    setStage("idle");
    setText({ final: "", interim: "" });
    setActivity(silentActivity());
    setNotice("");
    setChoices([]);
    setSelection("");
    setThread("");
    setLoading(false);
  }
  const option =
    choices.find((item) => item.service_id + item.model === selection) ??
    choices.find((item) => item.available && item.default_model) ??
    choices.find((item) => item.available);
  const active = stage !== "idle";
  const capturing = ["requesting", "connecting", "capturing"].includes(stage);
  const invalidate = useCallback(() => {
    generation.current++;
    client.current?.cancel();
    client.current = null;
  }, []);
  useLayoutEffect(() => {
    busy.current = false;
    held.current = false;
    gesture.current = null;
    return invalidate;
  }, [scope, invalidate]);
  function configure(next: boolean) {
    setOpen(next);
    if (next && !choices.length) {
      setLoading(true);
      setNotice("");
    }
  }
  useEffect(() => {
    if (!open || !getThread || !enabled || choices.length) return;
    const ticket = generation.current;
    let alive = true;
    void getThread()
      .then(async (id) => {
        const data = voiceOptionsSchema.parse(
          await api.get(
            `/assistant/nyxagent/dictation/options?conversation_id=${encodeURIComponent(id)}`,
          ),
        );
        if (!alive || generation.current !== ticket) return;
        setThread(id);
        setChoices(data.options);
        if (!data.options.some((item) => item.available))
          setNotice(
            "Dictation is unavailable for this conversation. Check your account access and billing settings.",
          );
      })
      .catch(() => {
        if (alive && generation.current === ticket)
          setNotice("Dictation options could not be loaded. Try again.");
      })
      .finally(() => {
        if (alive && generation.current === ticket) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [open, enabled, getThread, choices.length]);
  function start() {
    if (busy.current || disabled || !option?.available || !thread) return;
    busy.current = true;
    setApproved(true);
    begin();
    setNotice("");
    setText({ final: "", interim: "" });
    if (!held.current) setOpen(false);
    const ticket = ++generation.current;
    const next = new AssistantDictationClient({
      text: (value) => {
        if (generation.current === ticket) setText(value);
      },
      activity: (value) => {
        if (generation.current === ticket) setActivity(value);
      },
      state: (value) => {
        if (generation.current === ticket) setStage(value);
      },
      error: (message) => {
        if (generation.current === ticket) setNotice(message);
      },
    });
    client.current = next;
    const preferences: VoicePreferences = {
      service_id: option.service_id,
      connection_id: null,
      key_source: "platform",
      model: option.model,
      voice: null,
      language: null,
      input_mode: "push_to_talk",
      notify_on_completion: false,
    };
    void next.start(thread, preferences);
  }
  function stop() {
    setHolding(false);
    held.current = false;
    gesture.current = null;
    setOpen(false);
    client.current?.stop();
  }
  function cancel(restoreFocus = true) {
    if (restoreFocus) reviewClosed?.();
    invalidate();
    busy.current = false;
    setHolding(false);
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
    if (!active) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancel();
      }
    };
    const pause = () => {
      if (document.visibilityState === "hidden") stop();
    };
    window.addEventListener("keydown", escape);
    document.addEventListener("visibilitychange", pause);
    return () => {
      window.removeEventListener("keydown", escape);
      document.removeEventListener("visibilitychange", pause);
    };
  });
  const cancelIneligible = useEffectEvent(() => {
    setApproved(false);
    cancel(false);
  });
  useEffect(() => {
    if (!enabled || disabled) cancelIneligible();
  }, [enabled, disabled]);
  const pressHandlers = {
    onPointerDown: (event: React.PointerEvent<HTMLButtonElement>) => {
      if (mode !== "hold" || event.button !== 0 || !option?.available) return;
      event.preventDefault();
      held.current = true;
      setHolding(true);
      gesture.current = { pointer: event.pointerId };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      start();
    },
    onPointerUp: () => {
      if (mode === "hold" && held.current) stop();
    },
    onPointerCancel: () => {
      if (mode === "hold" && held.current) stop();
    },
    onLostPointerCapture: () => {
      if (mode === "hold" && held.current) stop();
    },
    onKeyDown: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (mode === "hold" && (event.key === " " || event.key === "Enter")) {
        event.preventDefault();
        if (!event.repeat) {
          held.current = true;
          setHolding(true);
          gesture.current = { key: event.key };
          start();
        }
      }
    },
    onKeyUp: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (mode === "hold" && (event.key === " " || event.key === "Enter")) {
        event.preventDefault();
        stop();
      }
    },
  };
  const button = enabled ? (
    <div className="flex items-center">
      <Button
        type="button"
        size="icon"
        variant="ghost"
        className={active ? "text-primary" : undefined}
        onMouseEnter={() => {
          if (!active) configure(true);
        }}
        aria-label={capturing ? "Stop dictation" : "Dictate a message"}
        aria-pressed={capturing}
        disabled={disabled || stage === "finalizing" || stage === "review"}
        {...(approved ? pressHandlers : {})}
        onClick={(event) => {
          if (mode === "hold" && approved && event.detail > 0) return;
          if (capturing) stop();
          else if (approved && mode === "toggle") start();
          else configure(true);
        }}
      >
        <Mic className="size-4" />
      </Button>
      <Popover open={open && (!active || holding)} onOpenChange={configure}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label="Configure dictation"
            onClick={(event) => {
              event.preventDefault();
              configure(true);
            }}
            disabled={disabled || active}
            className="w-6"
          >
            <ChevronDown className="size-3" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          side="top"
          align="end"
          onOpenAutoFocus={(event) => event.preventDefault()}
          className="max-w-[calc(100vw-2rem)] space-y-3 text-12"
        >
          <p className="font-medium">Dictate into your draft</p>
          <p className="text-muted-foreground">
            Transcription by OpenAI. Review before inserting; nothing is sent
            automatically.
          </p>
          <label className="block">
            Microphone control
            <select
              aria-label="Dictation control"
              value={mode}
              onChange={(event) => {
                const value = event.target.value as typeof mode;
                setMode(value);
                setApproved(false);
                try {
                  localStorage.setItem(
                    `nyxid:dictation-mode:${scope.split(":")[0]}`,
                    value,
                  );
                } catch {
                  /* Preference storage is optional. */
                }
              }}
              className="mt-1 h-8 w-full rounded-lg border border-input bg-background px-2"
            >
              <option value="toggle">Click to start / stop</option>
              <option value="hold">Hold to dictate</option>
            </select>
          </label>
          {choices.length > 1 && (
            <label className="block">
              Dictation source
              <select
                aria-label="Dictation source"
                value={option ? option.service_id + option.model : ""}
                onChange={(event) => {
                  setSelection(event.target.value);
                  setApproved(false);
                }}
                className="mt-1 h-8 w-full rounded-lg border border-input bg-background px-2"
              >
                {choices.map((item) => (
                  <option
                    key={item.service_id + item.model}
                    value={item.service_id + item.model}
                    disabled={!item.available}
                  >
                    {item.model_label} · Platform key
                  </option>
                ))}
              </select>
            </label>
          )}
          {option && (
            <p className="text-muted-foreground">
              {[option.pricing, ...(option.pricing?.components ?? [])]
                .filter(
                  (price) =>
                    price?.sync_status === "synced" &&
                    ["voice_seconds", "requests"].includes(price.metric),
                )
                .map(
                  (price) =>
                    `${price!.credits_per_unit} credits per ${price!.metric === "requests" ? "start" : "input audio second"}`,
                )
                .join(" + ") || "Tariff unavailable"}
              , paid by you. Fractional input seconds carry across turns; the
              final fraction is waived. Maximum 2 minutes.
            </p>
          )}
          {loading && <p role="status">Loading dictation options…</p>}
          {notice && (
            <p role="alert" className="text-destructive">
              {notice}
            </p>
          )}
          <Button
            type="button"
            variant="primary"
            disabled={!option?.available || loading || disabled}
            {...pressHandlers}
            onClick={(event) => {
              if (mode === "toggle" || event.detail === 0) start();
            }}
          >
            {mode === "hold" ? "Hold to dictate" : "Start dictation"}
          </Button>
        </PopoverContent>
      </Popover>
    </div>
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
  ) : null;
  return { button, review, active, activity, cancel };
}
