/* eslint react-refresh/only-export-components: off -- Local review page entry. */
import "../src/i18n";
import "../src/app.css";
import { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { Dice5, Pause, Play, RotateCcw, ArrowLeft } from "lucide-react";
import { Button } from "../src/components/ui/button";
import { AvatarRenderer } from "../src/components/assistant/avatar/avatar-renderer";
import { VoiceRippleBanner } from "../src/components/assistant/avatar/voice-ripple-banner";
import { EMOTES } from "../../shared/agent-avatars/emotes";
import {
  randomAvatar,
  avatarChoiceLabel,
  AVATAR_CATALOGUE,
  type AvatarConfig,
} from "../../shared/agent-avatars/model";
import { translateUi } from "../src/i18n/use-ui-translation";

function appearance(): AvatarConfig {
  const draft = randomAvatar(Math.random, "nyxbot");
  const choices = Object.fromEntries(
    Object.entries(draft.choices).filter(([key]) => !key.startsWith("react_")),
  );
  return {
    ...draft,
    choices: {
      ...choices,
      motion: "automatic",
      treatment: "gradient-face",
      background: "treatment",
      frame: "tile",
    },
  };
}
function EmotePreview() {
  const [avatars, setAvatars] = useState(() => EMOTES.map(appearance));
  const [playing, setPlaying] = useState(true);
  const [same, setSame] = useState(false);
  const [replay, setReplay] = useState(0);
  const [pixels, setPixels] = useState(192);
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [palette, setPalette] = useState("nyx");
  const [voicePhase, setVoicePhase] = useState("listening");
  const voices = useMemo(
    () => ({
      ...avatars[0]!,
      choices: {
        ...avatars[0]!.choices,
        ripple_palette: palette,
        surface: "flat",
        motion: playing ? "automatic" : "still",
      },
    }),
    [avatars, palette, playing],
  );
  return (
    <main className="min-h-screen bg-background text-foreground">
      <div className="mx-auto max-w-6xl space-y-8 px-5 py-8 sm:px-8">
        <a
          href="/assistant?mock=1&nyxbot=1&avatars=1"
          className="inline-flex items-center gap-2 type-secondary text-text-tertiary"
        >
          <ArrowLeft className="size-4" aria-hidden />
          {translateUi("Back to assistant")}
        </a>
        <header className="space-y-3">
          <p className="type-group-label text-text-tertiary">Nyx</p>
          <h1 className="type-page-title">{translateUi("Emote preview")}</h1>
          <p className="max-w-2xl type-body text-text-secondary">
            {translateUi(
              "Every emote uses the same pixel grid for the face, eyes and props.",
            )}
          </p>
        </header>
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-hairline bg-card p-4">
          <Button
            onClick={() => {
              setAvatars(EMOTES.map(appearance));
              setReplay((n) => n + 1);
            }}
          >
            <Dice5 className="size-4" aria-hidden />
            {translateUi("Randomise avatars")}
          </Button>
          <Button variant="outline" onClick={() => setPlaying(!playing)}>
            {playing ? (
              <Pause className="size-4" aria-hidden />
            ) : (
              <Play className="size-4" aria-hidden />
            )}
            {translateUi(playing ? "Pause" : "Play")}
          </Button>
          <Button variant="ghost" onClick={() => setReplay((n) => n + 1)}>
            <RotateCcw className="size-4" aria-hidden />
            {translateUi("Replay all")}
          </Button>
          <label className="flex items-center gap-2 type-secondary">
            <input
              type="checkbox"
              checked={same}
              onChange={(event) => setSame(event.target.checked)}
            />
            {translateUi("Use the same avatar")}
          </label>
          <label className="flex items-center gap-2 type-secondary">
            {translateUi("Preview size")}
            <select
              aria-label={translateUi("Preview size")}
              value={pixels}
              onChange={(event) => setPixels(Number(event.target.value))}
              className="rounded-lg border border-hairline bg-background px-3 py-2"
            >
              <option value={192}>{translateUi("Full scene")}</option>
              <option value={112}>{translateUi("Agent details")}</option>
              <option value={56}>{translateUi("Chat header")}</option>
            </select>
          </label>
          <Button
            variant="ghost"
            onClick={() => {
              const next = theme === "dark" ? "light" : "dark";
              document.documentElement.classList.replace(
                `theme-${theme}`,
                `theme-${next}`,
              );
              setTheme(next);
            }}
          >
            {translateUi(theme === "dark" ? "Light theme" : "Dark theme")}
          </Button>
        </div>
        <section
          aria-label={translateUi("Emotes")}
          className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3"
        >
          {EMOTES.map((emote, index) => {
            const base = avatars[same ? 0 : index]!;
            const surface = "flat";
            const config = { ...base, choices: { ...base.choices, surface } };
            return (
              <article
                key={emote.id}
                data-emote-card={emote.id}
                className="flex flex-col items-center gap-5 rounded-2xl border border-hairline bg-card p-6 text-center"
              >
                <div className="flex h-48 w-full items-center justify-center">
                  <div style={{ width: pixels, height: pixels }}>
                    <AvatarRenderer
                      key={pixels}
                      config={config}
                      kind="nyxbot"
                      agentId={`gallery-${index}`}
                      mood={emote.id}
                      theme={theme}
                      pixels={pixels}
                      animated={playing}
                      eventKey={`replay:${replay}`}
                      label={translateUi(emote.label)}
                      className="block size-full"
                    />
                  </div>
                </div>
                <div className="space-y-1">
                  <h2 className="type-card-title">
                    {translateUi(emote.label)}
                  </h2>
                  <p className="type-secondary text-text-tertiary">
                    {translateUi(emote.description)}
                  </p>
                </div>
                <p className="type-secondary text-text-tertiary">
                  {[
                    avatarChoiceLabel("head", config.choices.head ?? "round"),
                    avatarChoiceLabel("eyes", config.choices.eyes ?? "tall"),
                    avatarChoiceLabel("palette", config.choices.palette!),
                    "Pixel art",
                  ]
                    .map((label) => translateUi(label))
                    .join(" · ")}
                </p>
              </article>
            );
          })}
        </section>
        <section className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h2 className="type-section-title">
              {translateUi("Voice previews")}
            </h2>
            <div className="flex flex-wrap gap-3">
              <label className="flex items-center gap-2 type-secondary">
                {translateUi("Voice phase")}
                <select
                  aria-label={translateUi("Voice phase")}
                  value={voicePhase}
                  onChange={(event) => setVoicePhase(event.target.value)}
                  className="rounded-lg border border-hairline bg-card px-3 py-2"
                >
                  <option value="listening">{translateUi("Listening")}</option>
                  <option value="speaking">{translateUi("Speaking")}</option>
                </select>
              </label>
              <label className="flex items-center gap-2 type-secondary">
                {translateUi("Voice colour")}
                <select
                  aria-label={translateUi("Voice colour")}
                  value={palette}
                  onChange={(event) => setPalette(event.target.value)}
                  className="rounded-lg border border-hairline bg-card px-3 py-2"
                >
                  {Object.entries(AVATAR_CATALOGUE.fields.ripple_palette).map(
                    ([id, entry]) => (
                      <option key={id} value={id}>
                        {translateUi(entry.label)}
                      </option>
                    ),
                  )}
                </select>
              </label>
            </div>
          </div>
          <p className="type-secondary text-text-tertiary">
            {translateUi("These previews do not use your microphone.")}
          </p>
          <div>
            <VoiceRippleBanner
              config={voices}
              kind="nyxbot"
              name="Nyx"
              status={translateUi(
                voicePhase === "listening" ? "Listening" : "Speaking",
              )}
              inputAccepting={playing && voicePhase === "listening"}
              outputPlaying={playing && voicePhase === "speaking"}
              outputLevel={0.6}
            />
          </div>
        </section>
      </div>
    </main>
  );
}
createRoot(document.getElementById("emote-preview")!).render(<EmotePreview />);
