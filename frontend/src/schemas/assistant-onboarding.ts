import { z } from "zod";

/** First-run NyxBot onboarding: interest answers and next-step cards. */
export const onboardingOutcomeSchema = z.enum(["completed", "skipped"]);

const choiceSchema = z.object({
  id: z.string(),
  label: z.string(),
  icon: z.string(),
  /** Topics only: who the topic is offered to. */
  work_styles: z.array(z.string()).optional(),
});

/** Clicking one sends `prompt` to NyxBot as the owner's message. */
export const nextStepSchema = z.object({
  id: z.string(),
  title: z.string(),
  description: z.string(),
  prompt: z.string(),
  icon: z.string(),
});

export const assistantOnboardingSchema = z.object({
  show_intro: z.boolean(),
  /** The owner hid the home's suggestions; the intro still offers them. */
  suggestions_hidden: z.boolean().default(false),
  intro_version: z.number(),
  profile: z
    .object({
      // Values unknown to this build are kept as plain strings.
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

export type AssistantOnboarding = z.infer<typeof assistantOnboardingSchema>;
export type OnboardingChoice = z.infer<typeof choiceSchema>;
export type NextStep = z.infer<typeof nextStepSchema>;

export interface OnboardingAnswers {
  readonly outcome: z.infer<typeof onboardingOutcomeSchema>;
  readonly topics: readonly string[];
  readonly work_style: string | null;
}
