import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import {
  Mic,
  MicOff,
  PhoneOff,
  Square,
  Volume2,
  VolumeX,
  Minimize2,
  Maximize2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAppForm } from "@/components/ui/form";
import { metricLabel } from "@/schemas/billing-metrics";
import { api } from "@/lib/api-client";
import { useAssistantVoice } from "@/hooks/use-assistant-voice";
import {
  voiceOptionsSchema,
  type VoiceOption,
  type VoicePreferences,
} from "@/schemas/assistant-voice";

function tariff(option: VoiceOption) {
  if (option.key_source === "own" && !option.pricing) {
    const tokens = option.reported_token_pricing;
    const extras = tokens
      ? [tokens, ...tokens.components].filter(
          (p) => p.sync_status === "synced" && p.metric.includes("tokens"),
        )
      : [];
    return `No NyxID ${extras.length ? "duration" : "voice"} charge; your provider's charges still apply${extras.map((p) => `; ${p.credits_per_unit} credits per reported ${metricLabel(p.metric, 1)}`).join("")}`;
  }
  const prices = option.pricing
    ? [option.pricing, ...option.pricing.components]
    : [];
  const duration = prices.find(
    (p) => p.metric === "voice_seconds" && p.sync_status === "synced",
  );
  const extras = prices.filter(
    (p) =>
      p.sync_status === "synced" &&
      p.metric !== "voice_seconds" &&
      (p.metric === "requests" ||
        (option.voice.protocol === "xai_realtime" &&
          p.metric.includes("tokens")) ||
        option.voice.billing_metrics.includes(
          p.metric as (typeof option.voice.billing_metrics)[number],
        )),
  );
  return duration
    ? `${duration.credits_per_unit} credits per second${extras.map((p) => ` plus ${p.credits_per_unit} credits per ${p.metric === "requests" ? "start" : metricLabel(p.metric, 1)}`).join("")}`
    : "Voice tariff unavailable";
}

/** Media remains mounted when minimized; starting always needs a gesture. */
export function VoicePanel({
  threadId,
  savedPreferences,
  onClose,
}: {
  threadId: string;
  savedPreferences?: VoicePreferences | null;
  onClose: () => void;
}) {
  const voice = useAssistantVoice(threadId);
  const [minimized, setMinimized] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [selected, setSelected] = useState("");
  const preferencesForm = useAppForm<
    Pick<VoicePreferences, "input_mode" | "voice" | "notify_on_completion">
  >({
    defaultValues: {
      input_mode: savedPreferences?.input_mode ?? "push_to_talk",
      voice: savedPreferences?.voice ?? null,
      notify_on_completion: savedPreferences?.notify_on_completion ?? false,
    },
  });
  const configuredInputMode = preferencesForm.watch("input_mode");
  const preferredVoice = preferencesForm.watch("voice");
  const notifyOnCompletion = preferencesForm.watch("notify_on_completion");
  const options = useQuery({
    queryKey: ["assistant-voice-options", threadId],
    queryFn: async () =>
      voiceOptionsSchema.parse(
        await api.get(
          `/assistant/nyxagent/voice/options?conversation_id=${encodeURIComponent(threadId)}`,
        ),
      ),
  });
  const choices = options.data?.options ?? [];
  const option =
    choices.find(
      (o) =>
        `${o.service_id}:${o.connection_id ?? o.key_source}:${o.model}` ===
        selected,
    ) ??
    (savedPreferences
      ? choices.find(
          (o) =>
            o.service_id === savedPreferences.service_id &&
            o.connection_id === savedPreferences.connection_id &&
            o.key_source === savedPreferences.key_source &&
            o.model === savedPreferences.model,
        )
      : undefined) ??
    choices.find((o) => o.available && o.default_model) ??
    choices.find((o) => o.available);
  const isGrok = option?.voice.protocol === "xai_realtime";
  const inputMode = isGrok ? "push_to_talk" : configuredInputMode;
  const voiceName =
    option?.voice.voices.find((v) => v.id === preferredVoice)?.id ??
    option?.voice.voices[0]?.id ??
    null;
  const active = voice.starting || voice.connected;
  const elapsed = voice.snapshot
    ? Math.max(
        0,
        Math.floor(
          ((voice.snapshot.session.closed_at
            ? Date.parse(voice.snapshot.session.closed_at)
            : now) -
            Date.parse(voice.snapshot.session.created_at)) /
            1000,
        ),
      )
    : 0;
  const timer = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  const status = voice.starting
    ? "Connecting"
    : voice.snapshot?.session.state === "active"
      ? "Connected"
      : voice.snapshot?.session.state === "closing"
        ? "Ending"
        : "Ready";
  const microphoneMuted = voice.snapshot?.session.muted ?? false;
  const selectionKey = option
    ? `${option.service_id}:${option.connection_id ?? option.key_source}:${option.model}`
    : "";
  async function start() {
    if (!option?.available) return;
    const preferences: VoicePreferences = {
      service_id: option.service_id,
      connection_id: option.connection_id,
      key_source: option.key_source,
      model: option.model,
      voice: voiceName,
      input_mode: inputMode,
      language: savedPreferences?.language ?? null,
      notify_on_completion: notifyOnCompletion,
    };
    setSettingsError(null);
    try {
      await api.put("/assistant/nyxagent/settings", { voice: preferences });
    } catch {
      setSettingsError(
        "Voice preferences could not be saved. Please try again.",
      );
      return;
    }
    if (isGrok) await voice.start(preferences, "xai_realtime");
    else await voice.start(preferences);
  }
  const micControl = (
    <Button
      variant="outline"
      aria-label={microphoneMuted ? "Unmute microphone" : "Mute microphone"}
      aria-pressed={microphoneMuted}
      disabled={voice.starting}
      onClick={() => {
        void voice.mute(!microphoneMuted);
      }}
    >
      {microphoneMuted ? (
        <MicOff className="h-4 w-4" />
      ) : (
        <Mic className="h-4 w-4" />
      )}
      {microphoneMuted ? "Unmute" : "Mute"}
    </Button>
  );
  const endControl = (
    <Button
      variant="destructive"
      onClick={() => {
        void voice.end();
        setMinimized(false);
      }}
    >
      <PhoneOff className="h-4 w-4" />
      End call
    </Button>
  );
  if (minimized && active)
    return (
      <section
        aria-label="Minimized voice call"
        className="fixed bottom-6 right-4 z-40 flex max-w-[calc(100vw-2rem)] flex-wrap items-center gap-2 rounded-xl border border-border bg-card p-3 shadow-lg"
      >
        <span role="status">{status}</span>
        <span aria-label="Call duration" className="font-mono tabular-nums">
          {timer}
        </span>
        {inputMode === "push_to_talk" && (
          <Button
            aria-label="Hold to talk"
            disabled={voice.starting || microphoneMuted}
            aria-pressed={voice.holding}
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              voice.hold(true);
            }}
            onPointerUp={() => voice.hold(false)}
            onPointerCancel={() => voice.hold(false)}
            onBlur={() => voice.hold(false)}
            onKeyDown={(e) => {
              if (e.key === " " || e.key === "Enter") {
                e.preventDefault();
                voice.hold(true);
              }
            }}
            onKeyUp={() => voice.hold(false)}
          >
            <Mic className="h-4 w-4" />
          </Button>
        )}
        {micControl}
        {endControl}
        <Button
          variant="ghost"
          aria-label="Restore voice panel"
          onClick={() => setMinimized(false)}
        >
          <Maximize2 className="h-4 w-4" />
        </Button>
        {voice.snapshot?.session.idle_warning && (
          <p role="status" className="w-full">
            Speak to keep this call open.
          </p>
        )}
        {voice.error && (
          <p role="alert" className="w-full text-destructive">
            {voice.error}
          </p>
        )}
      </section>
    );
  return (
    <section
      aria-label="Voice conversation"
      className="rounded-xl border border-border bg-card p-4 text-12"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-15 font-semibold">Talk to your assistant</h2>
        <span role="status">{status}</span>
        <span aria-label="Call duration" className="font-mono tabular-nums">
          {timer}
        </span>
        {active && (
          <Button
            variant="ghost"
            aria-label="Minimize voice call"
            onClick={() => {
              voice.hold(false);
              setMinimized(true);
            }}
          >
            <Minimize2 className="h-4 w-4" />
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            void voice.end();
            onClose();
          }}
        >
          Close
        </Button>
      </div>
      <p className="mt-2 text-muted-foreground">
        Audio goes to {isGrok ? "xAI through NyxID" : "OpenAI"}. NyxID keeps the
        transcript, never an audio recording. Muting keeps the call running and
        billed.
      </p>
      {voice.conflictSession && !active && (
        <div className="mt-4 rounded-lg border border-warning/40 p-3">
          <p role="status">A voice call is already active in this conversation.</p>
          <div className="mt-2 flex gap-2">
            {voice.conflictSession.resumable && (
              <Button onClick={() => voice.rejoin()}>Rejoin</Button>
            )}
            <Button variant="destructive" onClick={() => void voice.end()}>
              End call
            </Button>
          </div>
        </div>
      )}
      {isGrok && (
        <p className="mt-2 text-muted-foreground">
          Grok private beta: Hold to talk, with headphones recommended.
          Automatic speaker mode is awaiting verification.
        </p>
      )}
      {!active && (
        <div className="mt-4 flex flex-wrap gap-3">
          <label>
            Voice source
            <select
              aria-label="Voice source"
              value={selectionKey}
              onChange={(e) => setSelected(e.target.value)}
              className="ml-2 h-8 rounded-lg border border-input bg-background px-3"
            >
              {choices.map((o) => (
                <option
                  key={`${o.service_id}:${o.connection_id ?? o.key_source}:${o.model}`}
                  disabled={!o.available}
                  value={`${o.service_id}:${o.connection_id ?? o.key_source}:${o.model}`}
                >
                  {o.model_label} ·{" "}
                  {o.key_source === "own" ? "Your key" : "Platform key"}
                  {!o.available ? " (not available yet)" : ""}
                </option>
              ))}
            </select>
          </label>
          <label>
            Voice
            <select
              aria-label="Voice"
              value={voiceName ?? ""}
              onChange={(e) =>
                preferencesForm.setValue("voice", e.target.value)
              }
              className="ml-2 h-8 rounded-lg border border-input bg-background px-3"
            >
              {option?.voice.voices.map((v) => (
                <option key={v.id} value={v.id}>
                  {v.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Microphone
            <select
              aria-label="Microphone mode"
              value={inputMode}
              onChange={(e) =>
                preferencesForm.setValue(
                  "input_mode",
                  e.target.value as typeof inputMode,
                )
              }
              className="ml-2 h-8 rounded-lg border border-input bg-background px-3"
            >
              <option value="push_to_talk">Hold to talk</option>
              <option value="automatic" disabled={isGrok}>
                Automatic{isGrok ? " (not available in Grok beta)" : ""}
              </option>
            </select>
          </label>
        </div>
      )}
      {!active && (
        <label className="mt-3 flex items-center gap-2">
          <input
            type="checkbox"
            checked={notifyOnCompletion}
            onChange={(event) =>
              preferencesForm.setValue(
                "notify_on_completion",
                event.target.checked,
              )
            }
          />
          Notify when tasks finish after this call (no message preview)
        </label>
      )}
      {option && (
        <p className="mt-3 text-muted-foreground">
          {tariff(option)}. Paid by{" "}
          {option.billing_owner === "organization" ? "the organization" : "you"}
          . Assistant tasks are billed separately. Calls end after 30 minutes or
          3 minutes without user speech.
        </p>
      )}
      {settingsError && (
        <p role="alert" className="mt-3 text-destructive">
          {settingsError}
        </p>
      )}
      {(!option || !option.available) && options.isSuccess && (
        <p role="status" className="mt-3">
          Select an available voice source to start. Your saved choice may no
          longer be available.
        </p>
      )}
      {voice.error && (
        <p role="alert" className="mt-3 text-destructive">
          {voice.error}
        </p>
      )}
      {options.isError && (
        <p role="alert" className="mt-3 text-destructive">
          Voice options could not be loaded.
        </p>
      )}
      {voice.snapshot?.session.idle_warning && (
        <p role="status" className="mt-3 text-warning">
          Speak to keep this call open. It will end shortly.
        </p>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        {!active ? (
          <Button
            variant="primary"
            disabled={!option?.available || voice.starting}
            onClick={() => {
              void start();
            }}
          >
            <Mic className="h-4 w-4" />
            Start voice
          </Button>
        ) : (
          <>
            {inputMode === "push_to_talk" && (
              <Button
                aria-pressed={voice.holding}
                onPointerDown={(e) => {
                  e.currentTarget.setPointerCapture(e.pointerId);
                  voice.hold(true);
                }}
                onPointerUp={() => voice.hold(false)}
                onPointerCancel={() => voice.hold(false)}
                onBlur={() => voice.hold(false)}
                onKeyDown={(e) => {
                  if (e.key === " " || e.key === "Enter") {
                    e.preventDefault();
                    voice.hold(true);
                  }
                }}
                onKeyUp={() => voice.hold(false)}
              >
                <Mic className="h-4 w-4" />
                Hold to talk
              </Button>
            )}
            {micControl}
            <Button
              variant="outline"
              aria-label={
                voice.speakerMuted ? "Enable speaker" : "Mute speaker"
              }
              aria-pressed={voice.speakerMuted}
              onClick={() => voice.muteSpeaker(!voice.speakerMuted)}
            >
              {voice.speakerMuted ? (
                <VolumeX className="h-4 w-4" />
              ) : (
                <Volume2 className="h-4 w-4" />
              )}
              Speaker
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                void voice.resumeAudio();
              }}
            >
              <Volume2 className="h-4 w-4" />
              Resume audio
            </Button>
            {endControl}
          </>
        )}
      </div>
      <p className="mt-2 text-muted-foreground">
        If the assistant hears its own voice, use headphones or Hold to talk.
      </p>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        {(["user", "assistant"] as const).map((speaker) => (
          <div
            key={speaker}
            role="log"
            aria-live="polite"
            aria-label={
              speaker === "user" ? "Your captions" : "Assistant captions"
            }
            className="max-h-48 space-y-2 overflow-y-auto"
          >
            <h3 className="font-medium">
              {speaker === "user" ? "You" : "Assistant"}
            </h3>
            {voice.snapshot?.captions
              .filter((c) => c.speaker === speaker)
              .map((c) => (
                <p key={c.id}>
                  {c.text}
                  {c.sealed && !c.complete ? " (interrupted)" : ""}
                </p>
              ))}
          </div>
        ))}
      </div>
      <ul className="mt-3 space-y-2" aria-label="Voice tasks">
        {voice.snapshot?.tasks.map((t, i) => (
          <li key={t.id} className="flex items-center justify-between gap-2">
            <span className="min-w-0 truncate">
              {t.title?.trim() || `Request ${i + 1}`} · {({
                queued: "Queued",
                claimed: "Running",
                awaiting_confirmation: "Needs your OK",
                completed: "Done",
                cancelled: "Cancelled",
              } as Record<string, string>)[t.state] ?? t.state}
              {t.result_message_id ? (
                <a
                  className="ml-1 text-nyx-secondary-400 underline"
                  href={`#message-${t.result_message_id}`}
                >
                  result
                </a>
              ) : null}
            </span>
            {!["completed", "cancelled"].includes(t.state) && (
              <Button
                size="sm"
                variant="ghost"
                aria-label={`Stop request ${i + 1}`}
                onClick={() => {
                  void voice.stopTask(t.id);
                }}
              >
                <Square className="h-3 w-3" />
                Stop
              </Button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
