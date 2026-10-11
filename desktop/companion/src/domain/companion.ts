import { z } from "zod";

export const COMPANION_SCHEMA_VERSION = 1 as const;
export const MEAL_IDS = ["breakfast", "lunch", "dinner"] as const;
export const RECOMMENDATION_MOODS = ["light", "balanced", "treat"] as const;
export const BUDGET_LEVELS = ["low", "everyday", "flexible"] as const;

export const mealIdSchema = z.enum(MEAL_IDS);
export const recommendationMoodSchema = z.enum(RECOMMENDATION_MOODS);
export const budgetSchema = z.enum(BUDGET_LEVELS);

export type MealId = z.infer<typeof mealIdSchema>;
export type RecommendationMood = z.infer<typeof recommendationMoodSchema>;
export type Budget = z.infer<typeof budgetSchema>;

const timeSchema = z
  .string()
  .regex(/^(?:[01]\d|2[0-3]):[0-5]\d$/, "Expected a 24-hour HH:MM time");

const dateKeySchema = z
  .string()
  .regex(/^\d{4}-\d{2}-\d{2}$/)
  .refine((value) => {
    const [year, month, day] = value.split("-").map(Number);
    if (year === undefined || month === undefined || day === undefined) {
      return false;
    }
    const date = new Date(Date.UTC(year, month - 1, day));
    return (
      date.getUTCFullYear() === year &&
      date.getUTCMonth() === month - 1 &&
      date.getUTCDate() === day
    );
  }, "Expected a calendar date");

const instantSchema = z.string().datetime({ offset: true });
const noControlCharacters = (value: string) =>
  !Array.from(value).some((character) => /\p{Cc}/u.test(character));

export function isValidTimezone(timezone: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: timezone }).format();
    return true;
  } catch {
    return false;
  }
}

export const timezoneSchema = z
  .string()
  .trim()
  .min(1)
  .max(64)
  .refine(noControlCharacters, "Control characters are not allowed")
  .refine(isValidTimezone, "Expected an IANA time zone");

export const mealScheduleSchema = z.object({
  id: mealIdSchema,
  label: z
    .string()
    .trim()
    .min(1)
    .max(48)
    .refine(noControlCharacters, "Control characters are not allowed"),
  time: timeSchema,
  enabled: z.boolean(),
});

const preferenceSchema = z
  .string()
  .trim()
  .min(1)
  .max(48)
  .refine(noControlCharacters, "Control characters are not allowed");

export const choiceIdSchema = z
  .string()
  .trim()
  .min(1)
  .max(160)
  .refine(noControlCharacters, "Control characters are not allowed");

export const companionSettingsSchema = z
  .object({
    companionName: z
      .string()
      .trim()
      .min(1)
      .max(80)
      .refine(noControlCharacters, "Control characters are not allowed"),
    userName: z
      .string()
      .trim()
      .max(80)
      .refine(noControlCharacters, "Control characters are not allowed"),
    timezone: timezoneSchema,
    budget: budgetSchema,
    dietary: z.array(preferenceSchema).max(12),
    avoid: z.array(preferenceSchema).max(20),
    quietMode: z.boolean(),
    onboardingComplete: z.boolean(),
    meals: z.array(mealScheduleSchema).length(MEAL_IDS.length),
  })
  .superRefine((settings, context) => {
    const ids = new Set(settings.meals.map((meal) => meal.id));
    for (const mealId of MEAL_IDS) {
      if (!ids.has(mealId)) {
        context.addIssue({
          code: "custom",
          message: `Missing ${mealId} schedule`,
          path: ["meals"],
        });
      }
    }
    for (const field of ["dietary", "avoid"] as const) {
      const normalized = settings[field].map((value) => value.toLowerCase());
      if (new Set(normalized).size !== normalized.length) {
        context.addIssue({
          code: "custom",
          message: `${field} may not contain duplicate entries`,
          path: [field],
        });
      }
    }
  });

export const historyActionSchema = z.enum(["accepted", "skipped", "disliked"]);

export const historyEntrySchema = z.object({
  mealId: mealIdSchema,
  dateKey: dateKeySchema,
  action: historyActionSchema,
  choiceId: choiceIdSchema.optional(),
  mood: recommendationMoodSchema.optional(),
  at: instantSchema,
});

export const activePromptSchema = z.object({
  mealId: mealIdSchema,
  dueAt: instantSchema,
});

export const companionRuntimeStateSchema = z.object({
  activePrompt: activePromptSchema.optional(),
  snoozedUntil: instantSchema.optional(),
  lastPromptKey: z.string().trim().min(1).max(96).optional(),
});

export const companionSnapshotSchema = z.object({
  schemaVersion: z.literal(COMPANION_SCHEMA_VERSION),
  settings: companionSettingsSchema,
  history: z.array(historyEntrySchema).max(120),
  runtime: companionRuntimeStateSchema,
});

export type MealSchedule = z.infer<typeof mealScheduleSchema>;
export type CompanionSettings = z.infer<typeof companionSettingsSchema>;
export type HistoryAction = z.infer<typeof historyActionSchema>;
export type HistoryEntry = z.infer<typeof historyEntrySchema>;
export type ActivePrompt = z.infer<typeof activePromptSchema>;
export type CompanionRuntimeState = z.infer<typeof companionRuntimeStateSchema>;
export type CompanionSnapshot = z.infer<typeof companionSnapshotSchema>;

const DEFAULT_MEALS: ReadonlyArray<MealSchedule> = [
  { id: "breakfast", label: "Breakfast", time: "08:00", enabled: true },
  { id: "lunch", label: "Lunch", time: "12:30", enabled: true },
  { id: "dinner", label: "Dinner", time: "18:30", enabled: true },
];

function systemTimezone(): string {
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  return timezone && isValidTimezone(timezone) ? timezone : "UTC";
}

export function createDefaultCompanionSettings(
  timezone = systemTimezone(),
): CompanionSettings {
  const safeTimezone = isValidTimezone(timezone) ? timezone : "UTC";
  return {
    companionName: "Nyx",
    userName: "",
    timezone: safeTimezone,
    budget: "everyday",
    dietary: [],
    avoid: [],
    quietMode: false,
    onboardingComplete: false,
    meals: DEFAULT_MEALS.map((meal) => ({ ...meal })),
  };
}

export function createDefaultCompanionSnapshot(
  timezone = systemTimezone(),
): CompanionSnapshot {
  return {
    schemaVersion: COMPANION_SCHEMA_VERSION,
    settings: createDefaultCompanionSettings(timezone),
    history: [],
    runtime: {},
  };
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function boundedString(
  value: unknown,
  fallback: string,
  maxLength: number,
): string {
  if (typeof value !== "string") {
    return fallback;
  }
  const normalized = Array.from(value.trim())
    .filter((character) => noControlCharacters(character))
    .slice(0, maxLength)
    .join("");
  return normalized || fallback;
}

function optionalBoundedString(
  value: unknown,
  maxLength: number,
): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }
  const normalized = Array.from(value.trim())
    .filter((character) => noControlCharacters(character))
    .slice(0, maxLength)
    .join("");
  return normalized || undefined;
}

function preferenceList(value: unknown, maximumEntries: number): string[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const item of value) {
    const candidate = optionalBoundedString(item, 48);
    if (!candidate || !noControlCharacters(candidate)) {
      continue;
    }
    const key = candidate.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      normalized.push(candidate);
    }
    if (normalized.length === maximumEntries) {
      break;
    }
  }
  return normalized;
}

function rawMealsById(value: unknown): Map<MealId, Record<string, unknown>> {
  const meals = new Map<MealId, Record<string, unknown>>();
  if (Array.isArray(value)) {
    for (const item of value) {
      const source = record(item);
      const parsedId = mealIdSchema.safeParse(source?.id);
      if (source && parsedId.success && !meals.has(parsedId.data)) {
        meals.set(parsedId.data, source);
      }
    }
    return meals;
  }

  const source = record(value);
  if (!source) {
    return meals;
  }
  for (const mealId of MEAL_IDS) {
    const meal = record(source[mealId]);
    if (meal) {
      meals.set(mealId, meal);
    }
  }
  return meals;
}

export function sanitizeCompanionSettings(
  value: unknown,
  fallback = createDefaultCompanionSettings(),
): CompanionSettings {
  const source = record(value) ?? {};
  const rawMeals = rawMealsById(source.meals);
  const fallbackMeals = new Map(fallback.meals.map((meal) => [meal.id, meal]));

  const meals = MEAL_IDS.map((mealId): MealSchedule => {
    const base =
      fallbackMeals.get(mealId) ??
      DEFAULT_MEALS.find((meal) => meal.id === mealId)!;
    const rawMeal = rawMeals.get(mealId) ?? {};
    const time = timeSchema.safeParse(rawMeal.time);
    return {
      id: mealId,
      label: boundedString(rawMeal.label, base.label, 48),
      time: time.success ? time.data : base.time,
      enabled:
        typeof rawMeal.enabled === "boolean" ? rawMeal.enabled : base.enabled,
    };
  });

  const timezone = timezoneSchema.safeParse(source.timezone);
  const budget = budgetSchema.safeParse(source.budget);
  const candidate = {
    companionName: boundedString(
      source.companionName,
      fallback.companionName,
      80,
    ),
    userName: boundedString(source.userName, fallback.userName, 80),
    timezone: timezone.success ? timezone.data : fallback.timezone,
    budget: budget.success ? budget.data : fallback.budget,
    dietary: preferenceList(source.dietary, 12),
    avoid: preferenceList(source.avoid, 20),
    quietMode:
      typeof source.quietMode === "boolean"
        ? source.quietMode
        : fallback.quietMode,
    onboardingComplete:
      typeof source.onboardingComplete === "boolean"
        ? source.onboardingComplete
        : fallback.onboardingComplete,
    meals,
  };

  return companionSettingsSchema.parse(candidate);
}

function migrateHistoryAction(value: unknown): HistoryAction | undefined {
  const direct = historyActionSchema.safeParse(value);
  if (direct.success) {
    return direct.data;
  }
  if (value === "completed" || value === "complete") {
    return "accepted";
  }
  if (value === "skip") {
    return "skipped";
  }
  if (value === "rejected") {
    return "disliked";
  }
  return undefined;
}

function sanitizeHistory(value: unknown): HistoryEntry[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const entries: HistoryEntry[] = [];
  for (const item of value.slice(-120)) {
    const source = record(item);
    const mealId = mealIdSchema.safeParse(source?.mealId ?? source?.meal);
    const action = migrateHistoryAction(source?.action ?? source?.outcome);
    const at = instantSchema.safeParse(source?.at ?? source?.occurredAt);
    if (!source || !mealId.success || !action || !at.success) {
      continue;
    }
    const fallbackDateKey = at.data.slice(0, 10);
    const dateKey = dateKeySchema.safeParse(source.dateKey ?? fallbackDateKey);
    if (!dateKey.success) {
      continue;
    }

    const choiceId = optionalBoundedString(source.choiceId, 160);
    const mood = recommendationMoodSchema.safeParse(source.mood);
    entries.push({
      mealId: mealId.data,
      dateKey: dateKey.data,
      action,
      ...(choiceId ? { choiceId } : {}),
      ...(mood.success ? { mood: mood.data } : {}),
      at: at.data,
    });
  }
  return entries;
}

function sanitizeRuntime(value: unknown): CompanionRuntimeState {
  const source = record(value) ?? {};
  const activePrompt = activePromptSchema.safeParse(source.activePrompt);
  const snoozedUntil = instantSchema.safeParse(source.snoozedUntil);
  const lastPromptKey = optionalBoundedString(source.lastPromptKey, 96);

  return {
    ...(activePrompt.success ? { activePrompt: activePrompt.data } : {}),
    ...(snoozedUntil.success ? { snoozedUntil: snoozedUntil.data } : {}),
    ...(lastPromptKey ? { lastPromptKey } : {}),
  };
}

/**
 * Reads both the current persisted shape and the small pre-v1 prototype shape.
 * Invalid fields recover independently, so one corrupt preference never resets
 * the rest of the user's schedule.
 */
export function parseCompanionSnapshot(
  value: unknown,
  timezone = systemTimezone(),
): CompanionSnapshot {
  const defaults = createDefaultCompanionSnapshot(timezone);
  const source = record(value) ?? {};
  const settingsSource = source.settings ?? source.preferences;

  const snapshot: CompanionSnapshot = {
    schemaVersion: COMPANION_SCHEMA_VERSION,
    settings: sanitizeCompanionSettings(settingsSource, defaults.settings),
    history: sanitizeHistory(source.history),
    runtime: sanitizeRuntime(source.runtime),
  };
  return companionSnapshotSchema.parse(snapshot);
}

export const migrateCompanionSnapshot = parseCompanionSnapshot;
