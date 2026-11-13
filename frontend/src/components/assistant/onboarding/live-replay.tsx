import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import { ArrowUp } from "lucide-react";
import { ChatMessageBubble } from "@/components/assistant/chat-message";
import { NyxAgentAcknowledgementCard } from "@/components/assistant/nyxagent-acknowledgement-card";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { useMediaQuery } from "@/hooks/use-media-query";
import { cn } from "@/lib/utils";

/*
 * Live, scripted replays for the intro: the real chat message bubbles and
 * action card, driven by a timeline of sample data. They follow the user's
 * theme, text size and density, and never touch real conversations.
 */

import {
  EMPTY,
  RESEARCH,
  PAUSE_AFTER_MS,
  replayScript,
  type Beat,
  type Scene,
  type LiveReplayKind,
} from "../../../../../shared/onboardingReplay";
export type { LiveReplayKind } from "../../../../../shared/onboardingReplay";

/** Play `beats` in a loop; reduced motion shows the finished scene. */
function useScript(beats: readonly Beat[], still: boolean): Scene {
  const [scene, setScene] = useState<Scene>(() =>
    still ? beats.reduce((current, beat) => beat.apply(current), EMPTY) : EMPTY,
  );
  useEffect(() => {
    if (still) return;
    let index = 0;
    let timer = 0;
    const play = () => {
      const beat = beats[index];
      if (!beat) {
        index = 0;
        timer = window.setTimeout(play, PAUSE_AFTER_MS);
        return;
      }
      index += 1;
      timer = window.setTimeout(() => {
        setScene((current) => beat.apply(current));
        play();
      }, beat.wait);
    };
    play();
    return () => window.clearTimeout(timer);
  }, [beats, still]);
  return scene;
}

export function LiveReplay({
  kind,
  topics = [],
  label,
}: {
  readonly kind: LiveReplayKind;
  /** The chief-of-staff replay shows an example for the first picked topic. */
  readonly topics?: readonly string[];
  /** What the replay shows, for assistive tech. */
  readonly label: string;
}) {
  useUiTranslation();
  const still = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [beats] = useState(() => replayScript(kind, topics));
  const scene = useScript(beats, still);
  return (
    <figure className="overflow-hidden rounded-xl border border-border bg-background shadow-sm">
      <figcaption className="sr-only">{label}</figcaption>
      <div aria-hidden="true" className="pointer-events-none select-none">
        <div className="flex items-center gap-2 border-b border-border/50 bg-sidebar px-3 py-2">
          <AgentAvatar agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }} size="sm" />
          <span className="type-row-title text-foreground">{translateUi("NyxBot")}</span>
        </div>
        <div className="flex h-[18.75rem] flex-col justify-end gap-3 overflow-hidden px-4 pt-3">
          {scene.messages.map((entry) => (
            <div key={entry.id} className="flex flex-col gap-3">
              <div className="flex animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none">
                <ChatMessageBubble
                  message={entry}
                  interactiveCards={false}
                  feedbackEnabled={false}
                />
              </div>
              {scene.specialist?.after === entry.id ? (
                <div className="flex items-center gap-2 pl-[1.875rem] text-13 text-muted-foreground animate-in fade-in-0 duration-300 motion-reduce:animate-none">
                  <AgentAvatar agent={RESEARCH} size="xs" />
                  {scene.specialist.state === "working" ? (
                    <span className="flex items-center gap-1.5">
                      <span className="h-1.5 w-1.5 rounded-full bg-success motion-safe:animate-pulse" />{translateUi("Research is working on it")}</span>
                  ) : (
                    <span>{translateUi("Research replied to NyxBot")}</span>
                  )}
                </div>
              ) : null}
              {scene.card?.after === entry.id ? (
                <div className="animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none">
                  <NyxAgentAcknowledgementCard
                    acknowledgement={scene.card.row}
                    deciding={false}
                    onDecision={() => Promise.resolve()}
                  />
                </div>
              ) : null}
            </div>
          ))}
          {scene.working ? (
            <p className="flex items-center gap-1.5 pl-[1.875rem] text-13 text-text-tertiary">
              <span className="h-1.5 w-1.5 rounded-full bg-nyx-500 motion-safe:animate-pulse" />
              {scene.working}
            </p>
          ) : null}
        </div>
        <div className="p-3">
          <div className="flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2 text-16">
            <span className={cn("min-w-0 flex-1 truncate", scene.typed ? "text-foreground" : "text-text-tertiary")}>
              {scene.typed || translateUi("Message NyxBot")}
              {scene.typed ? <span className="ml-px inline-block h-[1em] w-px translate-y-[0.15em] bg-foreground motion-safe:animate-pulse" /> : null}
            </span>
            <span
              className={cn(
                "flex h-6 w-6 shrink-0 items-center justify-center rounded-full transition-colors",
                scene.typed
                  ? "nyx-gradient-vivid text-white"
                  : "bg-overlay text-text-tertiary",
              )}
            >
              <ArrowUp className="size-3.5" />
            </span>
          </div>
        </div>
      </div>
    </figure>
  );
}
