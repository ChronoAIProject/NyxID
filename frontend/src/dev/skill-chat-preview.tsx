import { useEffect, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { ArrowUp, Plus } from "lucide-react";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import {
  SkillTemplateCard,
  type SkillTemplateOutline,
} from "@/components/assistant/skill-template-card";
import {
  SkillRunChecklist,
  type SkillRunStep,
  type SkillRunStepStatus,
} from "@/components/assistant/skill-run-checklist";
import { UserMessageBubble } from "@/components/ui/user-message-bubble";
import { Button } from "@/components/ui/button";
import { MISSING_REPORT, loadSkillTemplateReport } from "./skill-template-report";
import "@/app.css";

// Local-only mock of a NyxBot chat invoking skills. Outlines come from the
// template check's report (the real parser output); the conversation and the
// run progress are illustrative, and nothing here talks to a backend.
interface ReportEntry {
  readonly name: string;
  readonly kind: "skill" | "skillset";
  readonly version: string;
  readonly members: readonly string[];
  readonly outline: SkillTemplateOutline | null;
}

type Scene = "suggest" | "named" | "running" | "needs-you" | "done";
const SCENES: ReadonlyArray<{ readonly value: Scene; readonly label: string }> = [
  { value: "suggest", label: "1 · Suggested from a task" },
  { value: "named", label: "2 · Named by you" },
  { value: "running", label: "3 · Running" },
  { value: "needs-you", label: "4 · Needs you" },
  { value: "done", label: "5 · Done" },
];

const NYXBOT = { id: "nyxbot", name: "NyxBot", kind: "nyxbot" } as const;

function User({ children }: { readonly children: string }) {
  return (
    <div className="ml-auto flex min-w-0 max-w-[90%] flex-col items-end gap-1 sm:max-w-[78%]">
      <UserMessageBubble className="max-w-full text-16 [overflow-wrap:anywhere]">{children}</UserMessageBubble>
    </div>
  );
}

function Assistant({ children }: { readonly children: ReactNode }) {
  return (
    <article className="flex items-start gap-2.5">
      <div className="min-w-0 w-full space-y-3 pt-0.5">{children}</div>
    </article>
  );
}

function Working({ label }: { readonly label: string }) {
  return (
    <div className="flex min-h-5 items-center gap-1.5 text-13 text-muted-foreground">
      <span aria-hidden className="inline-block h-1.5 w-1.5 motion-safe:animate-pulse rounded-full bg-nyx-secondary-400" />
      <span role="status">{label}</span>
    </div>
  );
}

function Started({ name }: { readonly name: string }) {
  return <p role="status" className="px-[7px] text-13 text-muted-foreground">You started {name}</p>;
}

/** Map the skillset's outline steps to run states, with NyxBot's notes. */
function runSteps(
  outline: SkillTemplateOutline,
  states: readonly SkillRunStepStatus[],
  details: Readonly<Record<number, string>>,
  substeps?: { readonly at: number; readonly outline: SkillTemplateOutline; readonly states: readonly SkillRunStepStatus[] },
): SkillRunStep[] {
  return outline.steps.map((step, index) => ({
    id: step.id,
    title: step.title,
    status: states[index] ?? "pending",
    uses: step.uses ? { name: step.uses.name, version: step.uses.version } : null,
    detail: details[index] ?? null,
    substeps:
      substeps && substeps.at === index
        ? substeps.outline.steps.map((sub, subIndex) => ({
            id: sub.id,
            title: sub.title,
            status: substeps.states[subIndex] ?? "pending",
          }))
        : undefined,
  }));
}

function Transcript({ scene, entries }: { readonly scene: Scene; readonly entries: readonly ReportEntry[] }) {
  const find = (name: string) => entries.find((entry) => entry.name === name);
  const desk = find("phone-front-desk-sg");
  const sms = find("sms-send-twilio");
  const voice = find("voice-agent-telnyx");
  if (!desk?.outline || !sms?.outline || !voice?.outline) {
    return <p className="type-body text-destructive">The report is missing the example skills.</p>;
  }
  const ask = (
    <User>
      I run a small salon in Singapore. Can you set up something that answers our phone when we're busy, takes
      bookings and texts people a confirmation?
    </User>
  );
  const card = (started: boolean) => (
    <SkillTemplateCard
      name={desk.name}
      version={desk.version}
      kind="skillset"
      members={desk.members}
      outline={desk.outline}
      needsAttach
      started={started}
      onStart={() => undefined}
      onDismiss={() => undefined}
    />
  );
  const suggestion = (started: boolean) => (
    <Assistant>
      <TextBlock text="There's a skillset made for this. Here's the plan in plain steps. Nothing is set up until you start it, and I'll still check with you before anything that costs money or reaches a customer." />
      {card(started)}
    </Assistant>
  );
  const firstDone = "+65 6018 2245 reserved on Telnyx. Singapore documents submitted.";
  switch (scene) {
    case "suggest":
      return (
        <>
          {ask}
          {suggestion(false)}
        </>
      );
    case "named":
      return (
        <>
          <User>Use sms-send-twilio to text +65 9123 4567 that their 3pm booking tomorrow is confirmed.</User>
          <Assistant>
            <TextBlock text="That's the **sms-send-twilio** skill. Here's what it does. Start it and I'll show you the exact text before it goes out." />
            <SkillTemplateCard
              name={sms.name}
              version={sms.version}
              kind="skill"
              outline={sms.outline}
              onStart={() => undefined}
              onDismiss={() => undefined}
            />
          </Assistant>
        </>
      );
    case "running":
      return (
        <>
          {ask}
          {suggestion(true)}
          <Started name={desk.name} />
          <Assistant>
            <TextBlock text="Started **phone-front-desk-sg**. Your Singapore number is reserved, so I'm creating the voice agent now." />
            <SkillRunChecklist
              name={desk.name}
              version={desk.version}
              kind="skillset"
              elapsed="0:42"
              onStop={() => undefined}
              steps={runSteps(
                desk.outline,
                ["done", "running"],
                { 0: firstDone, 1: "Writing its instructions and greeting." },
                { at: 1, outline: voice.outline, states: ["done", "running"] },
              )}
            />
            <Working label="Writing the agent's instructions · 0:42" />
          </Assistant>
        </>
      );
    case "needs-you":
      return (
        <>
          <Started name={desk.name} />
          <Assistant>
            <SkillRunChecklist
              name={desk.name}
              version={desk.version}
              kind="skillset"
              onStop={() => undefined}
              steps={runSteps(
                desk.outline,
                ["done", "done", "needs_you"],
                {
                  0: firstDone,
                  1: "Assistant \"Front desk\" created with your greeting.",
                  2: "Pick the voice your callers will hear.",
                },
              )}
            />
            <TextBlock text={"The voice agent is ready. Step 3 needs your pick. All three speak English, and each has one more language:\n\n1. **Sarah**, warm, English and Malay\n2. **Mei**, calm, English and Mandarin\n3. **Daniel**, bright, English and Tamil\n\nReply with a number, or ask me to play a sample."} />
          </Assistant>
        </>
      );
    case "done":
      return (
        <>
          <Started name={desk.name} />
          <Assistant>
            <SkillRunChecklist
              name={desk.name}
              version={desk.version}
              kind="skillset"
              steps={runSteps(
                desk.outline,
                ["done", "done", "done", "done", "done", "done", "done"],
                {
                  0: "+65 6018 2245 on Telnyx.",
                  1: "Assistant \"Front desk\" answers every call.",
                  2: "Mei, English and Mandarin.",
                  3: "Bookings, Orders, Follow-ups and Availability in Notion.",
                  4: "Confirmations from your Twilio number.",
                  5: "Stripe payment links for takeaway orders.",
                  6: "3 test calls passed: a booking, a paid order and a follow-up.",
                },
              )}
            />
            <TextBlock text={"Your front desk is live on **+65 6018 2245**. It answers when nobody picks up within four rings.\n\nNew bookings and orders appear in Notion, and anything it couldn't handle lands in **Follow-ups** for your team."} />
          </Assistant>
        </>
      );
  }
}

function RunBar({ scene }: { readonly scene: Scene }) {
  if (scene !== "running" && scene !== "needs-you") return null;
  const asking = scene === "needs-you";
  return (
    <div className="mb-2 flex items-center gap-2 rounded-lg border border-hairline bg-overlay/35 px-3 py-1.5 text-13 text-muted-foreground">
      <span className="font-mono text-text-tertiary">phone-front-desk-sg</span>
      <span aria-hidden>·</span>
      <span>{asking ? "Step 3 of 7" : "Step 2 of 7"}</span>
      <span aria-hidden>·</span>
      <span className={asking ? "text-warning" : "skill-step-shimmer"}>
        {asking ? "Choose the voice: needs you" : "Create the voice agent"}
      </span>
    </div>
  );
}

function Composer({ scene }: { readonly scene: Scene }) {
  return (
    <div className="shrink-0 bg-background px-4 pb-4 sm:px-6">
      <div className="mx-auto w-full max-w-[758px] pt-2">
        <RunBar scene={scene} />
        <div className="rounded-xl border border-hairline bg-card px-3 py-2">
          <p className="type-composer min-h-8 py-1 text-text-tertiary">Message NyxBot</p>
          <div className="flex items-center justify-between">
            <span className="flex size-7 items-center justify-center rounded-full border border-hairline text-muted-foreground">
              <Plus aria-hidden className="size-4" />
            </span>
            <span className="flex size-7 items-center justify-center rounded-full bg-foreground text-background">
              <ArrowUp aria-hidden className="size-4" />
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

export function SkillChatPreview() {
  const params = new URLSearchParams(window.location.search);
  const [scene, setScene] = useState<Scene>(
    SCENES.some((option) => option.value === params.get("scene")) ? (params.get("scene") as Scene) : "suggest",
  );
  const [theme, setTheme] = useState<"dark" | "light">(params.get("theme") === "light" ? "light" : "dark");
  const [entries, setEntries] = useState<readonly ReportEntry[] | null>(null);
  const [error, setError] = useState("");
  const toolbar = params.get("toolbar") !== "0";

  useEffect(() => {
    document.documentElement.classList.remove("theme-dark", "theme-light");
    document.documentElement.classList.add(`theme-${theme}`);
  }, [theme]);

  useEffect(() => {
    loadSkillTemplateReport<ReportEntry[]>()
      .then(setEntries)
      .catch(() => setError(MISSING_REPORT));
  }, []);

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      {toolbar ? (
        <div className="flex flex-wrap items-center gap-1.5 border-b border-hairline px-4 py-2">
          <span className="type-secondary-emphasis mr-2 text-muted-foreground">Local preview</span>
          {SCENES.map((option) => (
            <Button
              key={option.value}
              size="sm"
              variant={scene === option.value ? "default" : "ghost"}
              aria-pressed={scene === option.value}
              onClick={() => setScene(option.value)}
            >
              {option.label}
            </Button>
          ))}
          <span className="flex-1" />
          <Button size="sm" variant="outline" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
            {theme === "dark" ? "Light theme" : "Dark theme"}
          </Button>
        </div>
      ) : null}
      <header className="flex shrink-0 items-center gap-3 px-4 py-3 sm:px-6">
        <div className="mx-auto flex w-full max-w-[758px] items-center gap-3">
          <AgentAvatar agent={NYXBOT} size="lg" />
          <div className="min-w-0">
            <p className="type-card-title text-foreground">NyxBot</p>
            <p className="type-secondary text-text-tertiary">Front desk for the salon</p>
          </div>
        </div>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto px-4 sm:px-6">
        <div className="mx-auto flex w-full max-w-[758px] flex-col gap-4 py-4">
          {error ? <p role="alert" className="type-body text-destructive">{error}</p> : null}
          {entries ? <Transcript scene={scene} entries={entries} /> : null}
        </div>
      </main>
      <Composer scene={scene} />
    </div>
  );
}

if (import.meta.env.DEV) {
  const container = document.getElementById("root");
  if (container) {
    const root = import.meta.hot?.data.root ?? createRoot(container);
    if (import.meta.hot) import.meta.hot.data.root = root;
    root.render(<SkillChatPreview />);
  }
}
