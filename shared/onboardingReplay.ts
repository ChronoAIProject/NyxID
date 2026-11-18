/** Portable fixture timeline shared by the web and native onboarding replays.
 * No transport, credentials or native/browser APIs are involved. */
export interface ReplayMessage {
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly content: string;
  readonly timestamp: number;
  readonly status: "complete";
  readonly continuation_of?: string;
}
export interface ReplayAcknowledgement {
  id: string;
  kind: "action";
  status: "pending" | "allowed";
  summary: string;
  decider: "user";
  decided_by: string | null;
  reason: null;
  service_slug: null;
  service_name: null;
  tool_name: null;
  created_at: string;
  decided_at: string | null;
  expires_at: string;
}
export type LiveReplayKind = "chief-of-staff" | "specialists" | "control";
export interface Scene {
  readonly typed: string;
  readonly messages: readonly ReplayMessage[];
  readonly working: string | null;
  /** The specialist's status and the card sit after the message they follow. */
  readonly specialist: {
    readonly after: string;
    readonly state: "working" | "done";
  } | null;
  readonly card: {
    readonly after: string;
    readonly row: ReplayAcknowledgement;
  } | null;
}

export type Beat = {
  readonly wait: number;
  readonly apply: (scene: Scene) => Scene;
};

export const EMPTY: Scene = {
  typed: "",
  messages: [],
  working: null,
  specialist: null,
  card: null,
};
export const RESEARCH = {
  id: "intro-research",
  name: "research",
  display_name: "Research",
  kind: "specialist" as const,
};

let sequence = 0;
function message(
  role: "user" | "assistant",
  content: string,
  extra: Partial<ReplayMessage> = {},
): ReplayMessage {
  sequence += 1;
  return {
    id: `intro-${sequence}`,
    role,
    content,
    timestamp: Date.now(),
    status: "complete",
    ...extra,
  };
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
    apply: (scene) => ({
      ...scene,
      typed: "",
      messages: [...scene.messages, message("user", text)],
    }),
  });
  return beats;
}

function reply(content: string, wait = 1100): Beat[] {
  return [
    { wait: 250, apply: (scene) => ({ ...scene, working: "Working…" }) },
    {
      wait,
      apply: (scene) => ({
        ...scene,
        working: null,
        messages: [...scene.messages, message("assistant", content)],
      }),
    },
  ];
}

const SAMPLES: Readonly<
  Partial<Record<string, { ask: string; reply: string }>>
> = {
  inbox: {
    ask: "What needs me in my inbox?",
    reply:
      "Three emails need you today:\n\n- **Dana**: contract redlines (I drafted a reply)\n- **Stripe**: a payout failed\n- **Lee**: lunch on Thursday?",
  },
  calendar: {
    ask: "Plan my day",
    reply:
      "Three meetings and two free hours:\n\n- **10:00** Design review\n- **14:00** Investor call (prep notes ready)\n\nI moved your focus block to 15:30.",
  },
  customers: {
    ask: "How did the front desk do today?",
    reply:
      "14 customer chats answered and 2 sent to you:\n\n- A refund request over the limit\n- A question about bulk pricing",
  },
  sales: {
    ask: "Any new leads this week?",
    reply:
      "Five new leads, two look ready to buy:\n\n- **Acme Co**: asked for a demo\n- **Northwind**: wants pricing for 20 seats",
  },
  coding: {
    ask: "Any pull requests waiting on me?",
    reply:
      "Two reviews are waiting:\n\n- **#412** Fix login redirect (small)\n- **#415** Billing export (needs a test)",
  },
  research: {
    ask: "What happened in AI agents this week?",
    reply:
      "Three stories worth your time:\n\n- A new open standard for agent tools\n- Two big launches in coding agents\n\nWant the full brief with sources?",
  },
};
const DEFAULT_SAMPLE = {
  ask: "Catch me up on today",
  reply:
    "Here's your day:\n\n- Two meetings, the first at **10:00**\n- Three emails need a reply\n- Your weekly report is due Friday",
};

const lastId = (scene: Scene) => scene.messages.at(-1)?.id ?? "";

export function chiefScript(topics: readonly string[]): Beat[] {
  const sample =
    topics.map((topic) => SAMPLES[topic]).find(Boolean) ?? DEFAULT_SAMPLE;
  return [
    { wait: 500, apply: () => EMPTY },
    ...ask(sample.ask),
    ...reply(sample.reply, 1300),
  ];
}

export const SPECIALISTS: Beat[] = [
  { wait: 500, apply: () => EMPTY },
  ...ask("Ask the researcher for urgent issues"),
  ...reply("I'll hand this to Research and report back.", 900),
  {
    wait: 500,
    apply: (scene) => ({
      ...scene,
      specialist: { after: lastId(scene), state: "working" },
    }),
  },
  {
    wait: 1800,
    apply: (scene) => ({
      ...scene,
      specialist: scene.specialist && { ...scene.specialist, state: "done" },
    }),
  },
  ...reply(
    "Research found 3 urgent issues: **#12**, **#15** and **#18**. Want me to assign them?",
    900,
  ),
];

function card(status: ReplayAcknowledgement["status"]): ReplayAcknowledgement {
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

export const CONTROL: Beat[] = [
  { wait: 500, apply: () => EMPTY },
  ...ask("Delete agent key ci-bot"),
  ...reply("Deleting a key can't be undone, so I need your OK first.", 900),
  {
    wait: 400,
    apply: (scene) => ({
      ...scene,
      card: { after: lastId(scene), row: card("pending") },
    }),
  },
  {
    wait: 1900,
    apply: (scene) => ({
      ...scene,
      card: scene.card && { ...scene.card, row: card("allowed") },
    }),
  },
  {
    wait: 700,
    apply: (scene) => ({
      ...scene,
      messages: [
        ...scene.messages,
        message("user", "", {
          continuation_of: "00000000-0000-4000-8000-000000000001",
        }),
      ],
    }),
  },
  ...reply("Done: the agent key **ci-bot** is deleted.", 900),
];

export const PAUSE_AFTER_MS = 3500;

export function replayScript(
  kind: LiveReplayKind,
  topics: readonly string[] = [],
): readonly Beat[] {
  return kind === "chief-of-staff"
    ? chiefScript(topics)
    : kind === "specialists"
      ? SPECIALISTS
      : CONTROL;
}
export function finalScene(beats: readonly Beat[]) {
  return beats.reduce((scene, beat) => beat.apply(scene), EMPTY);
}
