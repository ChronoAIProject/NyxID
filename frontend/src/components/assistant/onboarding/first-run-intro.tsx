import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Check, ShieldCheck, Sparkles, UsersRound } from "lucide-react";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { useSaveOnboarding } from "@/hooks/use-assistant-onboarding";
import { useAuthStore } from "@/stores/auth-store";
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
import { preloadReplays } from "./replay-clips";

/** The screens after the greeting; the progress bar counts these. */
const STEPS = ["interests", "chief", "specialists", "control", "next"] as const;
type Step = "welcome" | (typeof STEPS)[number];
/** Leaving pills fade out before the shared ones move. */
const PILL_EXIT_MS = 140;

function reducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/**
 * The first-run NyxBot intro: a greeting for first-timers, a light, skippable
 * interest picker, three short screens on how NyxBot and specialists work,
 * then one-click next steps ranked by the answers. Finishing or skipping
 * saves the answers once.
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
  const firstName = useAuthStore((auth) => auth.user?.display_name?.trim().split(/\s+/)[0]);
  // Only a first-timer is greeted; a replay starts at the questions.
  const flow: readonly Step[] = entryPoint === "first_run" ? ["welcome", ...STEPS] : STEPS;
  const LAST = flow.length - 1;
  const [index, setIndex] = useState(0);
  const [topics, setTopics] = useState<readonly string[]>(() => state.profile?.topics ?? []);
  const [workStyle, setWorkStyle] = useState<string | null>(() => state.profile?.work_style ?? null);
  // The audience the topic list shows; it follows `workStyle` once leaving
  // pills have faded, while the choice itself is selected at once.
  const [audience, setAudience] = useState<string>(() => state.profile?.work_style ?? "solo");
  const [nextSteps, setNextSteps] = useState<readonly NextStep[] | null>(null);
  const [exiting, setExiting] = useState<ReadonlySet<string>>(() => new Set());
  const styleSwitch = useRef<number | undefined>(undefined);
  const saved = useRef<string | undefined>(undefined);
  const openedAt = useRef(0);
  const body = useRef<HTMLDivElement>(null);
  const moved = useRef(false);
  const step: Step = flow[index] ?? "interests";

  useEffect(() => {
    if (!open) return;
    preloadReplays();
    saved.current = undefined;
    openedAt.current = Date.now();
    setIndex(0);
    setNextSteps(null);
    setTopics(state.profile?.topics ?? []);
    setWorkStyle(state.profile?.work_style ?? null);
    setAudience(state.profile?.work_style ?? "solo");
    void capture({ name: "ui.dialog_opened", props: { dialog_id: "nyxbot_intro", entry_point: entryPoint } });
    // Only a fresh open resets the intro; later state refreshes keep progress.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Keep focus inside the intro when the focused control leaves with its step.
  useEffect(() => {
    if (!moved.current) return;
    body.current?.focus({ preventScroll: true });
  }, [index]);

  // Saves when the answers differ from the last save, so revisiting a step
  // and changing a pick after reaching the end is kept too.
  function persist(outcome: "completed" | "skipped") {
    const answers = { outcome, topics, work_style: workStyle };
    const key = JSON.stringify(answers);
    if (saved.current === key) return Promise.resolve(undefined);
    saved.current = key;
    return save.mutateAsync(answers).catch(() => {
      // Retried on the next visit: the server still shows the intro.
      saved.current = undefined;
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

  function goTo(target: number) {
    if (target < 0 || target > LAST || target === index) return;
    moved.current = true;
    setIndex(target);
    if (target === LAST) {
      // The current suggestions show at once; the save refreshes them.
      void persist("completed").then((result) => {
        if (result) setNextSteps(result.recommendations);
      });
    }
  }

  function next() {
    if (index >= LAST) return;
    void capture({
      name: "ui.dialog_step_completed",
      props: { dialog_id: "nyxbot_intro", step: index, total_steps: flow.length },
    });
    goTo(index + 1);
  }

  function back() {
    if (index > 0 && flow[index - 1] !== "welcome") goTo(index - 1);
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
  const offered = new Set(topicsFor(state.topics, audience).map((topic) => topic.id));

  // Switching audience runs in order: pills that leave fade out in place,
  // then the shared pills glide to their new places and new ones fade in.
  function chooseWorkStyle(id: string) {
    const next = workStyle === id ? null : id;
    const nextAudience = next ?? "solo";
    const kept = new Set(topicsFor(state.topics, nextAudience).map((topic) => topic.id));
    setWorkStyle(next);
    setTopics((current) => current.filter((topic) => kept.has(topic)));
    const apply = () => {
      styleSwitch.current = undefined;
      setExiting(new Set());
      setAudience(nextAudience);
    };
    window.clearTimeout(styleSwitch.current);
    const leaving = new Set([...offered].filter((topic) => !kept.has(topic)));
    if (!leaving.size || reducedMotion()) {
      apply();
      return;
    }
    setExiting(leaving);
    styleSwitch.current = window.setTimeout(apply, PILL_EXIT_MS);
  }
  useEffect(() => () => window.clearTimeout(styleSwitch.current), []);

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
          {step === "welcome" ? null : (
            <Progress
              index={(STEPS as readonly string[]).indexOf(step)}
              onSelect={(position) => goTo(flow.indexOf(STEPS[position] ?? "interests"))}
            />
          )}
          <div
            key={step}
            ref={body}
            tabIndex={-1}
            className="flex-1 space-y-4 outline-none px-0 pt-4 pb-2 animate-in fade-in-0 slide-in-from-right-3 duration-300 motion-reduce:animate-none md:px-6"
          >
            {step === "welcome" ? (
              <Welcome firstName={firstName} onStart={next} onSkip={skip} />
            ) : step === "interests" ? (
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
                <ChoiceGrid
                  label="Topics"
                  choices={state.topics}
                  shown={offered}
                  exiting={exiting}
                  selected={topics}
                  onToggle={toggleTopic}
                  multi
                />
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
                <NextStepCards steps={nextSteps ?? state.recommendations} onStart={start} />
              </StepHeading>
            )}
          </div>
          {step === "welcome" ? null : (
          <div className="mt-auto flex items-center gap-2 border-t border-border/50 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] md:px-6">
            {index < LAST ? (
              <Button variant="ghost" size="sm" onClick={skip}>
                Skip intro
              </Button>
            ) : null}
            <div className="ml-auto flex items-center gap-2">
              {index > 0 && index < LAST && flow[index - 1] !== "welcome" ? (
                <Button variant="outline" size="sm" onClick={back}>
                  Back
                </Button>
              ) : null}
              {index < LAST ? (
                <Button variant="primary" size="sm" onClick={next}>
                  {step === "interests" ? (topics.length || workStyle ? "Continue" : "Continue without picking") : "Next"}
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
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** The greeting a first-timer sees before any question. */
function Welcome({
  firstName,
  onStart,
  onSkip,
}: {
  readonly firstName: string | undefined;
  readonly onStart: () => void;
  readonly onSkip: () => void;
}) {
  const points = [
    { icon: Sparkles, text: "A chief of staff that works across your apps" },
    { icon: UsersRound, text: "Specialists with one job each, under your rules" },
    { icon: ShieldCheck, text: "Nothing sensitive happens without your say-so" },
  ];
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-6 py-10 text-center md:py-16">
      <div className="relative animate-in fade-in-0 zoom-in-90 duration-500 ease-out motion-reduce:animate-none">
        <span
          aria-hidden="true"
          className="absolute inset-0 -m-3 rounded-full bg-nyx-500/15 blur-md motion-safe:animate-pulse"
        />
        <AgentAvatar agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }} size="xl" className="relative h-16 w-16" />
      </div>
      <div
        className="max-w-md space-y-2 animate-in fade-in-0 slide-in-from-bottom-2 duration-500 fill-mode-both motion-reduce:animate-none"
        style={{ animationDelay: "120ms" }}
      >
        <DialogTitle className="text-22 font-bold text-foreground sm:text-28" style={{ letterSpacing: "-0.03em" }}>
          {firstName ? `Welcome, ${firstName}` : "Welcome to NyxBot"}
        </DialogTitle>
        <DialogDescription id="nyxbot-intro-description" className="text-14 leading-relaxed text-muted-foreground">
          I'm NyxBot, your personal chief of staff. Give me a minute and I'll show you how we'll work together.
        </DialogDescription>
      </div>
      <ul className="w-full max-w-sm space-y-2 text-left">
        {points.map(({ icon: Icon, text }, position) => (
          <li
            key={text}
            className="flex items-center gap-3 rounded-lg border border-border bg-card px-3 py-2 shadow-sm animate-in fade-in-0 slide-in-from-bottom-1 duration-400 fill-mode-both motion-reduce:animate-none"
            style={{ animationDelay: `${260 + position * 90}ms` }}
          >
            <span
              aria-hidden="true"
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay text-foreground"
            >
              <Icon className="size-3.5" />
            </span>
            <span className="text-12 text-foreground">{text}</span>
          </li>
        ))}
      </ul>
      <div
        className="flex flex-col items-center gap-2 animate-in fade-in-0 duration-500 fill-mode-both motion-reduce:animate-none"
        style={{ animationDelay: "560ms" }}
      >
        <Button variant="primary" onClick={onStart}>
          Let's get started
        </Button>
        <Button variant="ghost" size="sm" onClick={onSkip}>
          Skip for now
        </Button>
      </div>
    </div>
  );
}

const STEP_LABELS: Readonly<Record<(typeof STEPS)[number], string>> = {
  interests: "Your picks",
  chief: "Chief of staff",
  specialists: "Specialists",
  control: "You stay in control",
  next: "First steps",
};

/** Every step is a target: click a bar to jump there. */
function Progress({
  index,
  onSelect,
}: {
  readonly index: number;
  readonly onSelect: (position: number) => void;
}) {
  return (
    <nav aria-label="Intro steps" className="-mx-0.75 flex items-center pt-1 md:mx-0 md:px-5.25 md:pt-4">
      {STEPS.map((step, position) => (
        <button
          key={step}
          type="button"
          title={STEP_LABELS[step]}
          aria-label={`Step ${position + 1} of ${STEPS.length}: ${STEP_LABELS[step]}`}
          aria-current={position === index ? "step" : undefined}
          onClick={() => onSelect(position)}
          // Bars sit as close as plain dashes: the assistant's minimum pointer
          // width does not apply; the tall padding keeps them easy to hit.
          className="group min-w-0! rounded-full px-0.75 py-2 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-input-focus"
        >
          <span
            aria-hidden="true"
            className={cn(
              "block h-1 rounded-full transition-all duration-300 motion-reduce:transition-none",
              position === index
                ? "w-6 bg-foreground"
                : position < index
                  ? "w-3 bg-muted-foreground group-hover:w-4 group-hover:bg-foreground"
                  : "w-3 bg-overlay-strong group-hover:w-4 group-hover:bg-muted-foreground",
            )}
          />
        </button>
      ))}
    </nav>
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

/**
 * Pills in a stable order. With `shown`, only those pills render; `exiting`
 * pills fade out in place first. Pills that stay glide to their new places
 * (measured against the list, so the dialog's own entrance cannot skew it),
 * and new pills fade in one after another.
 */
function ChoiceGrid({
  label,
  choices,
  shown,
  exiting,
  selected,
  onToggle,
  multi,
}: {
  readonly label: string;
  readonly choices: readonly OnboardingChoice[];
  readonly shown?: ReadonlySet<string>;
  readonly exiting?: ReadonlySet<string>;
  readonly selected: readonly string[];
  readonly onToggle: (id: string) => void;
  readonly multi?: boolean;
}) {
  const visible = shown ? choices.filter((choice) => shown.has(choice.id)) : choices;
  const nodes = useRef(new Map<string, HTMLElement>());
  const places = useRef(new Map<string, { x: number; y: number }>());
  const layoutKey = visible.map((choice) => choice.id).join(",");
  useLayoutEffect(() => {
    const animate = !reducedMotion();
    const next = new Map<string, { x: number; y: number }>();
    for (const [id, node] of nodes.current) {
      const place = { x: node.offsetLeft, y: node.offsetTop };
      next.set(id, place);
      const previous = places.current.get(id);
      if (!animate || !previous || (previous.x === place.x && previous.y === place.y)) continue;
      node.style.transition = "none";
      node.style.transform = `translate(${previous.x - place.x}px, ${previous.y - place.y}px)`;
      requestAnimationFrame(() => {
        node.style.transition = "transform 280ms cubic-bezier(0.2, 0, 0, 1)";
        node.style.transform = "";
      });
    }
    places.current = next;
  }, [layoutKey]);
  return (
    <div role={multi ? "group" : "radiogroup"} aria-label={label} className="relative flex flex-wrap gap-2">
      {visible.map((choice, position) => {
        const active = selected.includes(choice.id);
        const leaving = exiting?.has(choice.id) ?? false;
        return (
          <span
            key={choice.id}
            ref={(node) => {
              if (node) nodes.current.set(choice.id, node);
              else nodes.current.delete(choice.id);
            }}
            className="inline-flex animate-in fade-in-0 zoom-in-95 duration-200 ease-out fill-mode-both motion-reduce:animate-none"
            style={{ animationDelay: `${shown ? 60 + position * 20 : 0}ms` }}
          >
            <button
              type="button"
              role={multi ? "checkbox" : "radio"}
              aria-checked={active}
              disabled={leaving}
              onClick={() => onToggle(choice.id)}
              className={cn(
                "inline-flex items-center gap-2 whitespace-nowrap rounded-full border py-1 pr-3 pl-1 text-12 font-medium transition-[background-color,border-color,opacity,transform] duration-150 active:scale-95 motion-reduce:transition-none motion-reduce:active:scale-100",
                "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-input-focus",
                leaving && "scale-95 opacity-0",
                active
                  ? "border-input-focus bg-overlay-strong text-foreground"
                  : "border-hairline text-muted-foreground hover:border-hairline-strong hover:bg-overlay hover:text-foreground",
              )}
            >
              <ChoiceIcon name={choice.icon} active={active} />
              {choice.label}
              {active ? <Check aria-hidden="true" className="size-3" /> : null}
            </button>
          </span>
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
