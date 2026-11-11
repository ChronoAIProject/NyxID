import { z } from "zod";

/** First-run NyxBot onboarding, shared with the web app's contract. */
const choiceSchema = z.object({
  id: z.string(),
  label: z.string(),
  icon: z.string(),
  /** Topics only: who the topic is offered to. */
  work_styles: z.array(z.string()).optional(),
});

/** Starting one sends `prompt` to NyxBot as the owner's message. */
export const nextStepSchema = z.object({
  id: z.string(),
  title: z.string(),
  description: z.string(),
  prompt: z.string(),
  connect: z.boolean().optional(),
  icon: z.string(),
});

export const onboardingSchema = z.object({
  show_intro: z.boolean(),
  /** The owner hid the home's suggestions; the intro still offers them. */
  suggestions_hidden: z.boolean().default(false),
  intro_version: z.number(),
  profile: z
    .object({
      outcome: z.string(),
      topics: z.array(z.string()),
      work_style: z.string().nullable(),
      intro_version: z.number(),
      updated_at: z.string(),
    })
    .nullable(),
  topics: z.array(choiceSchema),
  work_styles: z.array(choiceSchema),
  recommendations: z.array(nextStepSchema),
});

export type Onboarding = z.infer<typeof onboardingSchema>;
export type OnboardingChoice = z.infer<typeof choiceSchema>;
export type NextStep = z.infer<typeof nextStepSchema>;

export type OnboardingAnswers = {
  outcome: "completed" | "skipped";
  topics: readonly string[];
  work_style: string | null;
};

export const onboardingKey = ["assistant", "onboarding"] as const;

/**
 * Feather names (the family lucide grew from) for the server's lucide icon
 * names; unknown icons get a star.
 */
const FEATHER: Record<string, string> = {
  "at-sign": "at-sign",
  calendar: "calendar",
  "chart-no-axes-combined": "bar-chart-2",
  code: "code",
  "credit-card": "credit-card",
  "file-text": "file-text",
  folder: "folder",
  headset: "headphones",
  mail: "mail",
  megaphone: "volume-2",
  "messages-square": "message-square",
  "pen-line": "edit-3",
  repeat: "repeat",
  search: "search",
  smartphone: "smartphone",
  sparkles: "star",
  store: "shopping-bag",
  sunrise: "sunrise",
  target: "target",
  user: "user",
  users: "users",
  zap: "zap",
};

export function featherIcon(icon: string): string {
  return FEATHER[icon] ?? "star";
}

/** Topics offered to a work style; topics without a list are offered to all. */
export function topicsFor(
  topics: readonly OnboardingChoice[],
  style: string,
): readonly OnboardingChoice[] {
  return topics.filter(
    (topic) => !topic.work_styles || topic.work_styles.includes(style),
  );
}

/** Live provider choices, fetched only when a setup suggestion is opened. */
export const suggestionSetupSchema = z.object({
  connected: z.boolean(),
  choices: z.array(z.object({ slug: z.string(), name: z.string(), scopes: z.array(z.string()) })),
  prompt: z.string(),
});
export type SuggestionSetup = z.infer<typeof suggestionSetupSchema>;
