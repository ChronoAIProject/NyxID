import { useState } from "react";
import { useMediaQuery } from "@/hooks/use-media-query";
import { CLIPS } from "./replay-clips";
import { ACCESS_MODE_DESCRIPTION, ACCESS_MODE_LABEL } from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import type { NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";

/*
 * Real assistant screens for the first-run intro: recorded replays of the
 * product (sample data only) and an access-mode explainer.
 */


function WindowBar({ title }: { readonly title: string }) {
  return (
    <div className="flex items-center gap-2 border-b border-border/50 bg-sidebar px-3 py-2">
      <span aria-hidden="true" className="flex gap-1">
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
      </span>
      <span className="truncate text-11 font-medium text-text-tertiary">{title}</span>
    </div>
  );
}

/**
 * A recorded replay of the real assistant (dev mock mode, fixture data;
 * regenerate with `scripts/record-onboarding-clips.mjs`). Loops silently like
 * a GIF: VP9 first, H.264 as the fallback, and a still frame while loading.
 * With reduced motion it waits for the play control instead.
 */
export function ReplayClip({
  clip,
  title,
  label,
}: {
  readonly clip: keyof typeof CLIPS;
  readonly title: string;
  /** What the replay shows, for assistive tech. */
  readonly label: string;
}) {
  const reduced = useMediaQuery("(prefers-reduced-motion: reduce)");
  const files = CLIPS[clip];
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-background shadow-sm">
      <WindowBar title={title} />
      <video
        key={clip}
        aria-label={label}
        poster={files.poster}
        className="block aspect-[1280/864] w-full bg-background object-cover"
        autoPlay={!reduced}
        controls={reduced}
        loop
        muted
        playsInline
        preload="auto"
        // Some browsers ignore autoplay on a freshly inserted element.
        onCanPlay={(event) => {
          if (!reduced && event.currentTarget.paused) void event.currentTarget.play().catch(() => undefined);
        }}
      >
        <source src={files.webm} type="video/webm" />
        <source src={files.mp4} type="video/mp4" />
      </video>
    </div>
  );
}

const MODES: readonly NyxAgentAccessMode[] = ["nyxmode", "ask_every_time", "autopilot"];

/** The access modes, explained on selection; it changes nothing. */
export function AccessModesPreview() {
  const [mode, setMode] = useState<NyxAgentAccessMode>("nyxmode");
  return (
    <div className="space-y-1.5">
      <div role="radiogroup" aria-label="Access modes" className="grid grid-cols-3 gap-1 rounded-lg border border-border bg-muted p-1">
        {MODES.map((value) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={mode === value}
            onClick={() => setMode(value)}
            className={cn(
              "rounded-md px-2 py-1 text-11 font-medium transition-colors motion-reduce:transition-none",
              mode === value
                ? "bg-card text-foreground shadow-sm"
                : "text-text-tertiary hover:text-foreground",
            )}
          >
            {ACCESS_MODE_LABEL[value]}
          </button>
        ))}
      </div>
      <p className="min-h-[2lh] text-11 leading-snug text-muted-foreground">
        {ACCESS_MODE_DESCRIPTION[mode]}
        {mode === "nyxmode" ? " This is the default." : ""}
      </p>
    </div>
  );
}
