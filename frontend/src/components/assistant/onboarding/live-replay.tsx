import { useEffect, useState } from "react";
import { ArrowUp } from "lucide-react";
import { ChatMessageBubble } from "@/components/assistant/chat-message";
import { NyxAgentAcknowledgementCard } from "@/components/assistant/nyxagent-acknowledgement-card";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { useMediaQuery } from "@/hooks/use-media-query";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { cn } from "@/lib/utils";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

/*
 * Live, scripted replays for the intro: the real chat message bubbles and
 * action card, driven by a timeline of sample data. They follow the user's
 * theme, text size and density, and never touch real conversations.
 */

interface Scene {
  readonly typed: string;
  readonly messages: readonly ChatMessage[];
  readonly working: string | null;
  /** The specialist's status and the card sit after the message they follow. */
  readonly specialist: { readonly after: string; readonly state: "working" | "done" } | null;
  readonly card: { readonly after: string; readonly row: NyxAgentAcknowledgement } | null;
}

type Beat = { readonly wait: number; readonly apply: (scene: Scene) => Scene };

const EMPTY: Scene = { typed: "", messages: [], working: null, specialist: null, card: null };
const RESEARCH = { id: "intro-research", name: "research", display_name: "Research", kind: "specialist" as const };

let sequence = 0;
function message(role: "user" | "assistant", content: string, extra: Partial<ChatMessage> = {}): ChatMessage {
  sequence += 1;
  return { id: `intro-${sequence}`, role, content, timestamp: Date.now(), status: "complete", ...extra };
}

/** Type `text` into the composer, then send it. */
function ask(text: string): Beat[] {
  const beats: Beat[] = [];
  for (let end = 2; end < text.length + 2; end += 2) {
    const typed = text.slice(0, end);
    beats.push({ wait: 55, apply: (scene) => ({ ...scene, typed }) });
  }
  beats.push({
    wait: 450,
    apply: (scene) => ({ ...scene, typed: "", messages: [...scene.messages, message("user", text)] }),
  });
  return beats;
}

function reply(content: string, wait = 1100): Beat[] {
  return [
    { wait: 250, apply: (scene) => ({ ...scene, working: "Working…" }) },
    {
      wait,
      apply: (scene) => ({ ...scene, working: null, messages: [...scene.messages, message("assistant", content)] }),
    },
  ];
}

const SAMPLES: Readonly<Partial<Record<string, { ask: string; reply: string }>>> = {
  inbox: {
    ask: "What needs me in my inbox?",
    reply: "Three emails need you today:\n\n- **Dana**: contract redlines (I drafted a reply)\n- **Stripe**: a payout failed\n- **Lee**: lunch on Thursday?",
  },
  calendar: {
    ask: "Plan my day",
    reply: "Three meetings and two free hours:\n\n- **10:00** Design review\n- **14:00** Investor call (prep notes ready)\n\nI moved your focus block to 15:30.",
  },
  customers: {
    ask: "How did the front desk do today?",
    reply: "14 customer chats answered and 2 sent to you:\n\n- A refund request over the limit\n- A question about bulk pricing",
  },
  sales: {
    ask: "Any new leads this week?",
    reply: "Five new leads, two look ready to buy:\n\n- **Acme Co**: asked for a demo\n- **Northwind**: wants pricing for 20 seats",
  },
  coding: {
    ask: "Any pull requests waiting on me?",
    reply: "Two reviews are waiting:\n\n- **#412** Fix login redirect (small)\n- **#415** Billing export (needs a test)",
  },
  research: {
    ask: "What happened in AI agents this week?",
    reply: "Three stories worth your time:\n\n- A new open standard for agent tools\n- Two big launches in coding agents\n\nWant the full brief with sources?",
  },
};
const DEFAULT_SAMPLE = {
  ask: "Catch me up on today",
  reply: "Here's your day:\n\n- Two meetings, the first at **10:00**\n- Three emails need a reply\n- Your weekly report is due Friday",
};

const lastId = (scene: Scene) => scene.messages.at(-1)?.id ?? "";

function chiefScript(topics: readonly string[]): Beat[] {
  const sample = topics.map((topic) => SAMPLES[topic]).find(Boolean) ?? DEFAULT_SAMPLE;
  return [{ wait: 500, apply: () => EMPTY }, ...ask(sample.ask), ...reply(sample.reply, 1300)];
}

const SPECIALISTS: Beat[] = [
  { wait: 500, apply: () => EMPTY },
  ...ask("Ask the researcher for urgent issues"),
  ...reply("I'll hand this to Research and report back.", 900),
  { wait: 500, apply: (scene) => ({ ...scene, specialist: { after: lastId(scene), state: "working" } }) },
  {
    wait: 1800,
    apply: (scene) => ({ ...scene, specialist: scene.specialist && { ...scene.specialist, state: "done" } }),
  },
  ...reply("Research found 3 urgent issues: **#12**, **#15** and **#18**. Want me to assign them?", 900),
];

function card(status: NyxAgentAcknowledgement["status"]): NyxAgentAcknowledgement {
  const now = new Date().toISOString();
  return {
    id: "00000000-0000-4000-8000-000000000001",
    kind: "action",
    status,
    summary: "Delete agent key 'ci-bot'",
    decider: "user",
    decided_by: status === "pending" ? null : "user",
    reason: null,
    service_slug: null,
    service_name: null,
    tool_name: null,
    created_at: now,
    decided_at: status === "pending" ? null : now,
    expires_at: new Date(Date.now() + 600_000).toISOString(),
  };
}

const CONTROL: Beat[] = [
  { wait: 500, apply: () => EMPTY },
  ...ask("Delete agent key ci-bot"),
  ...reply("Deleting a key can't be undone, so I need your OK first.", 900),
  { wait: 400, apply: (scene) => ({ ...scene, card: { after: lastId(scene), row: card("pending") } }) },
  {
    wait: 1900,
    apply: (scene) => ({ ...scene, card: scene.card && { ...scene.card, row: card("allowed") } }),
  },
  {
    wait: 700,
    apply: (scene) => ({
      ...scene,
      messages: [...scene.messages, message("user", "", { continuation_of: "00000000-0000-4000-8000-000000000001" })],
    }),
  },
  ...reply("Done: the agent key **ci-bot** is deleted.", 900),
];

const PAUSE_AFTER_MS = 3500;

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

export type LiveReplayKind = "chief-of-staff" | "specialists" | "control";

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
  const still = useMediaQuery("(prefers-reduced-motion: reduce)");
  const [beats] = useState(() =>
    kind === "chief-of-staff" ? chiefScript(topics) : kind === "specialists" ? SPECIALISTS : CONTROL,
  );
  const scene = useScript(beats, still);
  return (
    <figure className="overflow-hidden rounded-xl border border-border bg-background shadow-sm">
      <figcaption className="sr-only">{label}</figcaption>
      <div aria-hidden="true" className="pointer-events-none select-none">
        <div className="flex items-center gap-2 border-b border-border/50 bg-sidebar px-3 py-2">
          <AgentAvatar agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }} size="sm" />
          <span className="text-12 font-medium text-foreground">NyxBot</span>
        </div>
        <div className="flex h-[300px] flex-col justify-end gap-3 overflow-hidden px-4 pt-3">
          {scene.messages.map((entry) => (
            <div key={entry.id} className="flex flex-col gap-3">
              <div className="flex animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none">
                <ChatMessageBubble message={entry} interactiveCards={false} feedbackEnabled={false} />
              </div>
              {scene.specialist?.after === entry.id ? (
                <div className="flex items-center gap-2 pl-[30px] text-11 text-muted-foreground animate-in fade-in-0 duration-300 motion-reduce:animate-none">
                  <AgentAvatar agent={RESEARCH} size="xs" />
                  {scene.specialist.state === "working" ? (
                    <span className="flex items-center gap-1.5">
                      <span className="h-1.5 w-1.5 rounded-full bg-success motion-safe:animate-pulse" />
                      Research is working on it
                    </span>
                  ) : (
                    <span>Research replied to NyxBot</span>
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
            <p className="flex items-center gap-1.5 pl-[30px] text-11 text-text-tertiary">
              <span className="h-1.5 w-1.5 rounded-full bg-nyx-500 motion-safe:animate-pulse" />
              {scene.working}
            </p>
          ) : null}
        </div>
        <div className="p-3">
          <div className="flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2 text-12">
            <span className={cn("min-w-0 flex-1 truncate", scene.typed ? "text-foreground" : "text-text-tertiary")}>
              {scene.typed || "Message NyxBot"}
              {scene.typed ? <span className="ml-px inline-block h-[1em] w-px translate-y-[0.15em] bg-foreground motion-safe:animate-pulse" /> : null}
            </span>
            <span
              className={cn(
                "flex h-6 w-6 shrink-0 items-center justify-center rounded-full transition-colors",
                scene.typed ? "nyx-gradient-vivid text-white" : "bg-overlay text-text-tertiary",
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
