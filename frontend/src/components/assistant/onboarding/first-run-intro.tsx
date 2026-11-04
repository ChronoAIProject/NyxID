import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { useSaveOnboarding } from "@/hooks/use-assistant-onboarding";
import { capture } from "@/lib/telemetry";
import { cn } from "@/lib/utils";
import type {
  AssistantOnboarding,
  NextStep,
  OnboardingChoice,
} from "@/schemas/assistant-onboarding";
import { NextStepCards } from "./next-step-cards";
import { OnboardingIcon } from "./onboarding-icons";
import { AccessModesPreview, ReplayClip } from "./intro-previews";

const STEPS = ["interests", "chief", "specialists", "control", "next"] as const;
type Step = (typeof STEPS)[number];
const LAST = STEPS.length - 1;

/**
 * The first-run NyxBot intro: a light, skippable interest picker, three short
 * screens on how NyxBot and specialists work, then one-click next steps
 * ranked by the answers. Finishing or skipping saves the answers once.
 */
export function FirstRunIntro({
  open,
  state,
  entryPoint,
  onOpenChange,
  onStart,
  onTour,
}: {
  readonly open: boolean;
  readonly state: AssistantOnboarding;
  readonly entryPoint: "first_run" | "account_menu";
  readonly onOpenChange: (open: boolean) => void;
  /** Close the intro and send the step's prompt to NyxBot. */
  readonly onStart: (step: NextStep) => void;
  /** Close the intro and start the on-screen tour. */
  readonly onTour?: () => void;
}) {
  const save = useSaveOnboarding();
  const [index, setIndex] = useState(0);
  const [topics, setTopics] = useState<readonly string[]>(() => state.profile?.topics ?? []);
  const [workStyle, setWorkStyle] = useState<string | null>(() => state.profile?.work_style ?? null);
  const [steps, setSteps] = useState<readonly NextStep[] | null>(null);
  const saved = useRef(false);
  const openedAt = useRef(0);
  const body = useRef<HTMLDivElement>(null);
  const moved = useRef(false);
  const step: Step = STEPS[index] ?? "interests";

  useEffect(() => {
    if (!open) return;
    saved.current = false;
    openedAt.current = Date.now();
    setIndex(0);
    setSteps(null);
    setTopics(state.profile?.topics ?? []);
    setWorkStyle(state.profile?.work_style ?? null);
    void capture({ name: "ui.dialog_opened", props: { dialog_id: "nyxbot_intro", entry_point: entryPoint } });
    // Only a fresh open resets the intro; later state refreshes keep progress.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Keep focus inside the intro when the focused control leaves with its step.
  useEffect(() => {
    if (!moved.current) return;
    body.current?.focus({ preventScroll: true });
  }, [index]);

  function persist(outcome: "completed" | "skipped") {
    if (saved.current) return Promise.resolve(undefined);
    saved.current = true;
    return save
      .mutateAsync({ outcome, topics, work_style: workStyle })
      .catch(() => {
        // Retried on the next visit: the server still shows the intro.
        saved.current = false;
        return undefined;
      });
  }

  function skip() {
    void capture({
      name: "ui.dialog_abandoned",
      props: { dialog_id: "nyxbot_intro", final_step: index, duration_ms: Date.now() - openedAt.current },
    });
    // Closing a replay early keeps an earlier completion.
    const finished = index === LAST || state.profile?.outcome === "completed";
    void persist(finished ? "completed" : "skipped");
    onOpenChange(false);
  }

  function next() {
    if (index >= LAST) return;
    void capture({
      name: "ui.dialog_step_completed",
      props: { dialog_id: "nyxbot_intro", step: index, total_steps: STEPS.length },
    });
    const target = index + 1;
    moved.current = true;
    setIndex(target);
    if (target === LAST) {
      void persist("completed").then((result) => setSteps(result?.recommendations ?? state.recommendations));
    }
  }

  function back() {
    if (index > 0 && index < LAST) {
      moved.current = true;
      setIndex(index - 1);
    }
  }

  function start(nextStep: NextStep) {
    void persist("completed");
    onOpenChange(false);
    onStart(nextStep);
  }

  function tour() {
    void persist("completed");
    onOpenChange(false);
    onTour?.();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.altKey || event.metaKey || event.ctrlKey) return;
    if (event.key === "ArrowRight" && index < LAST) {
      event.preventDefault();
      next();
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      back();
    }
  }

  // Topics follow who NyxBot is for; personal topics until that is chosen.
  const audience = workStyle ?? "solo";
  const offered = topicsFor(state.topics, audience);

  function chooseWorkStyle(id: string) {
    const next = workStyle === id ? null : id;
    const kept = new Set(topicsFor(state.topics, next ?? "solo").map((topic) => topic.id));
    setWorkStyle(next);
    setTopics((current) => current.filter((topic) => kept.has(topic)));
  }

  function toggleTopic(id: string) {
    setTopics((current) =>
      current.includes(id) ? current.filter((topic) => topic !== id) : [...current, id],
    );
  }

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : skip())}>
      <DialogContent
        className="md:max-w-[640px] md:p-0 md:pt-0"
        onKeyDown={onKeyDown}
        // Start on the step itself so no chip looks pre-selected.
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          body.current?.focus({ preventScroll: true });
        }}
        aria-describedby="nyxbot-intro-description"
      >
        <div className="flex min-h-0 flex-1 flex-col md:min-h-[690px]">
          <Progress index={index} />
          <div
            key={step}
            ref={body}
            tabIndex={-1}
            className="flex-1 space-y-4 outline-none px-0 pt-4 pb-2 animate-in fade-in-0 slide-in-from-right-3 duration-300 motion-reduce:animate-none md:px-6"
          >
            {step === "interests" ? (
              <StepHeading
                title="Let's set NyxBot up for you"
                description="Tell us who it's for and what it should help with. We'll tailor your first suggestions and NyxBot will remember. You can skip this."
              >
                <Overline>Who is it for?</Overline>
                <ChoiceGrid
                  label="Who is it for"
                  choices={state.work_styles}
                  selected={workStyle ? [workStyle] : []}
                  onToggle={chooseWorkStyle}
                />
                <Overline>What should it help with?</Overline>
                <div
                  key={audience}
                  className="animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none"
                >
                  <ChoiceGrid
                    label="Topics"
                    choices={offered}
                    selected={topics}
                    onToggle={toggleTopic}
                    multi
                  />
                </div>
              </StepHeading>
            ) : step === "chief" ? (
              <StepHeading
                title="Meet NyxBot, your chief of staff"
                description="Ask in plain words. NyxBot works across the apps you connect, keeps track of what matters and remembers how you like things done."
              >
                <ReplayClip
                  clip="chief-of-staff"
                  title="NyxBot"
                  label="Replay: the owner asks NyxBot to set up a Telegram bot; NyxBot links the setup page and keeps watching until the bot exists."
                />
                <Points
                  items={[
                    "Chat here, on your phone, or in Telegram, Slack and other chat apps.",
                    "It uses only the services you connect, through NyxID.",
                  ]}
                />
              </StepHeading>
            ) : step === "specialists" ? (
              <StepHeading
                title="Specialists: experts with one job"
                description="Think of specialists as people who are great at one thing. Give each a fixed role and scope, so it does that job better than anyone. Work with them directly, or let NyxBot hand them the work."
              >
                <ReplayClip
                  clip="specialists"
                  title="NyxBot and a specialist"
                  label="Replay: the owner asks NyxBot for urgent issues; NyxBot hands the job to the researcher specialist, which reports back, and NyxBot answers."
                />
                <Points
                  items={[
                    "A role and scope you set: only the services and access you grant.",
                    "Chat with one directly, ask NyxBot to delegate, or put several in a crew.",
                  ]}
                />
              </StepHeading>
            ) : step === "control" ? (
              <StepHeading
                title="You stay in control"
                description="NyxBot asks before it does anything sensitive. You choose how often it asks, and you can change it any time in NyxBot settings."
              >
                <ReplayClip
                  clip="control"
                  title="NyxBot"
                  label="Replay: the owner asks NyxBot to delete an agent key; NyxBot shows a confirmation card, the owner allows it, and NyxBot finishes the deletion."
                />
                <AccessModesPreview />
              </StepHeading>
            ) : (
              <StepHeading
                title="Pick your first step"
                description="One click and NyxBot takes it from there. These stay on your home page until you're done with them. New here? Take a quick tour of the screen first."
              >
                {steps ? (
                  <NextStepCards steps={steps} onStart={start} />
                ) : (
                  <div className="grid gap-2" aria-label="Loading suggestions">
                    {[0, 1, 2].map((item) => (
                      <Skeleton key={item} className="h-14 rounded-lg" />
                    ))}
                  </div>
                )}
              </StepHeading>
            )}
          </div>
          <div className="mt-auto flex items-center gap-2 border-t border-border/50 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] md:px-6">
            {index < LAST ? (
              <Button variant="ghost" size="sm" onClick={skip}>
                Skip intro
              </Button>
            ) : null}
            <div className="ml-auto flex items-center gap-2">
              {index > 0 && index < LAST ? (
                <Button variant="outline" size="sm" onClick={back}>
                  Back
                </Button>
              ) : null}
              {index < LAST ? (
                <Button variant="primary" size="sm" onClick={next}>
                  {index === 0 ? (topics.length || workStyle ? "Continue" : "Continue without picking") : "Next"}
                </Button>
              ) : (
                <>
                  <Button variant="outline" size="sm" onClick={skip}>
                    Explore on my own
                  </Button>
                  {onTour ? (
                    <Button variant="primary" size="sm" onClick={tour}>
                      Show me around
                    </Button>
                  ) : null}
                </>
              )}
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function Progress({ index }: { readonly index: number }) {
  return (
    <div className="flex items-center gap-1.5 pt-1 md:px-6 md:pt-5" aria-label={`Step ${index + 1} of ${STEPS.length}`}>
      {STEPS.map((step, position) => (
        <span
          key={step}
          aria-hidden="true"
          className={cn(
            "h-1 rounded-full transition-all duration-300 motion-reduce:transition-none",
            position === index ? "w-6 bg-foreground" : position < index ? "w-3 bg-muted-foreground" : "w-3 bg-overlay-strong",
          )}
        />
      ))}
    </div>
  );
}

function StepHeading({
  title,
  description,
  children,
}: {
  readonly title: string;
  readonly description: string;
  readonly children: ReactNode;
}) {
  return (
    <>
      <div className="space-y-1 pr-8 md:pr-6">
        <DialogTitle className="text-15 font-semibold text-foreground">{title}</DialogTitle>
        <DialogDescription id="nyxbot-intro-description" className="text-12 leading-relaxed text-muted-foreground">
          {description}
        </DialogDescription>
      </div>
      {children}
    </>
  );
}

function Points({ items }: { readonly items: readonly string[] }) {
  return (
    <ul className="space-y-1.5">
      {items.map((item) => (
        <li key={item} className="flex gap-2 text-12 leading-snug text-muted-foreground">
          <Check aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-success" />
          {item}
        </li>
      ))}
    </ul>
  );
}

function ChoiceGrid({
  label,
  choices,
  selected,
  onToggle,
  multi,
}: {
  readonly label: string;
  readonly choices: readonly OnboardingChoice[];
  readonly selected: readonly string[];
  readonly onToggle: (id: string) => void;
  readonly multi?: boolean;
}) {
  return (
    <div role={multi ? "group" : "radiogroup"} aria-label={label} className="flex flex-wrap gap-2">
      {choices.map((choice) => {
        const active = selected.includes(choice.id);
        return (
          <button
            key={choice.id}
            type="button"
            role={multi ? "checkbox" : "radio"}
            aria-checked={active}
            onClick={() => onToggle(choice.id)}
            className={cn(
              "inline-flex items-center gap-2 rounded-full border py-1 pr-3 pl-1 text-12 font-medium transition-[background-color,border-color,transform] duration-150 active:scale-95 motion-reduce:transition-none motion-reduce:active:scale-100",
              "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-input-focus",
              active
                ? "border-input-focus bg-overlay-strong text-foreground"
                : "border-hairline text-muted-foreground hover:border-hairline-strong hover:bg-overlay hover:text-foreground",
            )}
          >
            <ChoiceIcon name={choice.icon} active={active} />
            {choice.label}
            {active ? <Check aria-hidden="true" className="size-3" /> : null}
          </button>
        );
      })}
    </div>
  );
}

/** The next-step cards' icon tile, sized for a chip. */
function ChoiceIcon({ name, active }: { readonly name: string; readonly active: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex h-6 w-6 shrink-0 items-center justify-center rounded-full border bg-overlay transition-colors motion-reduce:transition-none",
        active ? "border-hairline-strong text-foreground" : "border-hairline text-muted-foreground",
      )}
    >
      <OnboardingIcon name={name} className="size-3.5" />
    </span>
  );
}

function Overline({ children }: { readonly children: ReactNode }) {
  return (
    <p className="pt-1 text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary">{children}</p>
  );
}

/** Topics offered to a work style; topics without a list are offered to all. */
function topicsFor(topics: readonly OnboardingChoice[], style: string): readonly OnboardingChoice[] {
  return topics.filter((topic) => !topic.work_styles || topic.work_styles.includes(style));
}
