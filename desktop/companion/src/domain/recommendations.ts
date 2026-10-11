import type {
  Budget,
  CompanionSettings,
  HistoryEntry,
  MealId,
  RecommendationMood,
} from "./companion";

export interface LocalRecommendation {
  id: string;
  mealIds: ReadonlyArray<MealId>;
  moods: ReadonlyArray<RecommendationMood>;
  cost: Budget;
  prepMinutes: number;
  suitableFor: ReadonlyArray<string>;
  traits: ReadonlyArray<SoftPreference>;
  signals: ReadonlyArray<string>;
}

export type SoftPreference = "light" | "high-protein" | "low-oil" | "mild";

export type RecommendationReasonCode =
  | "catalog_baseline"
  | "mood_match"
  | "strong_mood_match"
  | "budget_fit"
  | "dietary_fit"
  | "preference_match"
  | "preference_miss"
  | "accepted_before"
  | "skipped_before"
  | "disliked_before"
  | "recent_repeat"
  | "daily_rotation";

export interface RecommendationReason {
  code: RecommendationReasonCode;
  points: number;
}

export interface ScoredRecommendation extends LocalRecommendation {
  score: number;
  reasons: ReadonlyArray<RecommendationReason>;
}

export type ExclusionReason = "budget" | "dietary" | "avoid";

export interface ExcludedRecommendation {
  id: string;
  reason: ExclusionReason;
}

export interface RecommendationSet {
  primary?: ScoredRecommendation;
  alternatives: ReadonlyArray<ScoredRecommendation>;
  excluded: ReadonlyArray<ExcludedRecommendation>;
}

export interface RecommendationInput {
  mealId: MealId;
  mood: RecommendationMood;
  settings: CompanionSettings;
  history: ReadonlyArray<HistoryEntry>;
  dateKey: string;
  limit?: number;
}

export const RECOMMENDATION_CATALOG: ReadonlyArray<LocalRecommendation> = [
  {
    id: "banana-oat-bowl",
    mealIds: ["breakfast"],
    moods: ["light", "balanced"],
    cost: "low",
    prepMinutes: 8,
    suitableFor: ["vegan", "vegetarian", "dairy-free"],
    traits: ["light", "low-oil", "mild"],
    signals: ["banana", "oats", "oat milk", "cinnamon"],
  },
  {
    id: "savory-egg-toast",
    mealIds: ["breakfast"],
    moods: ["balanced", "treat"],
    cost: "low",
    prepMinutes: 12,
    suitableFor: ["vegetarian", "dairy-free"],
    traits: ["high-protein", "mild"],
    signals: ["egg", "wheat", "spinach", "bread"],
  },
  {
    id: "berry-yogurt-crunch",
    mealIds: ["breakfast"],
    moods: ["light", "balanced"],
    cost: "everyday",
    prepMinutes: 5,
    suitableFor: ["vegetarian", "gluten-free"],
    traits: ["light", "mild"],
    signals: ["milk", "yogurt", "berries", "nuts"],
  },
  {
    id: "avocado-breakfast-tacos",
    mealIds: ["breakfast"],
    moods: ["treat", "balanced"],
    cost: "flexible",
    prepMinutes: 18,
    suitableFor: ["vegan", "vegetarian", "dairy-free", "gluten-free"],
    traits: ["mild"],
    signals: ["avocado", "corn", "beans", "tomato"],
  },
  {
    id: "lemon-lentil-soup",
    mealIds: ["lunch", "dinner"],
    moods: ["light", "balanced"],
    cost: "low",
    prepMinutes: 25,
    suitableFor: ["vegan", "vegetarian", "dairy-free", "gluten-free"],
    traits: ["light", "high-protein", "low-oil", "mild"],
    signals: ["lentils", "lemon", "carrot", "celery"],
  },
  {
    id: "sesame-tofu-rice",
    mealIds: ["lunch", "dinner"],
    moods: ["balanced", "light"],
    cost: "everyday",
    prepMinutes: 22,
    suitableFor: ["vegan", "vegetarian", "dairy-free", "gluten-free"],
    traits: ["high-protein", "mild"],
    signals: ["soy", "tofu", "sesame", "rice", "broccoli"],
  },
  {
    id: "herbed-chicken-rice",
    mealIds: ["lunch", "dinner"],
    moods: ["balanced"],
    cost: "everyday",
    prepMinutes: 28,
    suitableFor: ["dairy-free", "gluten-free"],
    traits: ["high-protein", "low-oil", "mild"],
    signals: ["chicken", "rice", "herbs", "cucumber"],
  },
  {
    id: "roasted-vegetable-pasta",
    mealIds: ["lunch", "dinner"],
    moods: ["balanced", "treat"],
    cost: "everyday",
    prepMinutes: 30,
    suitableFor: ["vegan", "vegetarian", "dairy-free"],
    traits: ["mild"],
    signals: ["wheat", "pasta", "tomato", "zucchini"],
  },
  {
    id: "salmon-greens-plate",
    mealIds: ["lunch", "dinner"],
    moods: ["light", "balanced"],
    cost: "flexible",
    prepMinutes: 24,
    suitableFor: ["pescatarian", "dairy-free", "gluten-free"],
    traits: ["light", "high-protein", "low-oil", "mild"],
    signals: ["fish", "salmon", "leafy greens", "lemon"],
  },
  {
    id: "mushroom-dumpling-bowl",
    mealIds: ["lunch", "dinner"],
    moods: ["treat"],
    cost: "everyday",
    prepMinutes: 20,
    suitableFor: ["vegan", "vegetarian", "dairy-free"],
    traits: [],
    signals: ["wheat", "soy", "mushroom", "sesame"],
  },
  {
    id: "steak-sweet-potato",
    mealIds: ["dinner"],
    moods: ["treat", "balanced"],
    cost: "flexible",
    prepMinutes: 35,
    suitableFor: ["dairy-free", "gluten-free"],
    traits: ["high-protein"],
    signals: ["beef", "sweet potato", "leafy greens"],
  },
  {
    id: "chickpea-flatbread",
    mealIds: ["lunch", "dinner"],
    moods: ["treat", "balanced"],
    cost: "low",
    prepMinutes: 18,
    suitableFor: ["vegan", "vegetarian", "dairy-free"],
    traits: ["high-protein", "mild"],
    signals: ["wheat", "chickpeas", "tomato", "herbs"],
  },
];

const budgetRank: Record<Budget, number> = {
  low: 0,
  everyday: 1,
  flexible: 2,
};

const hardDietAliases: Record<string, string> = {
  vegan: "vegan",
  vegetarian: "vegetarian",
  veggie: "vegetarian",
  pescatarian: "pescatarian",
  "gluten free": "gluten-free",
  glutenfree: "gluten-free",
  "dairy free": "dairy-free",
  dairyfree: "dairy-free",
};

const softPreferenceAliases: Record<string, SoftPreference> = {
  light: "light",
  "high protein": "high-protein",
  highprotein: "high-protein",
  "low oil": "low-oil",
  lowoil: "low-oil",
  mild: "mild",
};

const avoidAliases: Record<string, ReadonlyArray<string>> = {
  peanut: ["peanut"],
  peanuts: ["peanut"],
  花生: ["peanut"],
  nuts: ["nuts"],
  坚果: ["nuts"],
  dairy: ["milk", "yogurt"],
  milk: ["milk"],
  牛奶: ["milk"],
  乳制品: ["milk", "yogurt"],
  egg: ["egg"],
  eggs: ["egg"],
  鸡蛋: ["egg"],
  soy: ["soy", "tofu"],
  大豆: ["soy", "tofu"],
  豆腐: ["tofu"],
  sesame: ["sesame"],
  芝麻: ["sesame"],
  gluten: ["wheat"],
  wheat: ["wheat"],
  小麦: ["wheat"],
  麸质: ["wheat"],
  fish: ["fish", "salmon"],
  鱼: ["fish", "salmon"],
  三文鱼: ["salmon"],
  beef: ["beef"],
  牛肉: ["beef"],
  chicken: ["chicken"],
  鸡肉: ["chicken"],
  mushroom: ["mushroom"],
  蘑菇: ["mushroom"],
};

function normalized(value: string): string {
  return value
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[_-]+/g, " ")
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function hardDiets(settings: CompanionSettings): string[] {
  return settings.dietary.flatMap((diet) => {
    const mapped = hardDietAliases[normalized(diet)];
    return mapped ? [mapped] : [];
  });
}

function softPreferences(settings: CompanionSettings): SoftPreference[] {
  return settings.dietary.flatMap((preference) => {
    const mapped = softPreferenceAliases[normalized(preference)];
    return mapped ? [mapped] : [];
  });
}

const ignoredAvoidWords = new Set([
  "allergy",
  "allergic",
  "avoid",
  "free",
  "intolerance",
  "intolerant",
  "no",
]);

function avoidTokens(settings: CompanionSettings): Set<string> {
  const tokens = new Set<string>();
  for (const item of settings.avoid) {
    const normalizedItem = normalized(item);
    for (const token of normalizedItem.split(" ")) {
      if (token.length > 1 && !ignoredAvoidWords.has(token)) {
        tokens.add(token);
      }
    }
    for (const [alias, mapped] of Object.entries(avoidAliases)) {
      if (normalizedItem.includes(alias)) {
        mapped.forEach((token) => tokens.add(token));
      }
    }
  }
  return tokens;
}

function conflictsWithAvoids(
  recommendation: LocalRecommendation,
  avoids: ReadonlySet<string>,
): boolean {
  if (avoids.size === 0) {
    return false;
  }
  const signals = recommendation.signals.flatMap((signal) =>
    normalized(signal).split(" "),
  );
  return signals.some((signal) => avoids.has(signal));
}

function supportsDiet(
  recommendation: LocalRecommendation,
  diet: string,
): boolean {
  if (recommendation.suitableFor.includes(diet)) {
    return true;
  }
  if (diet === "vegetarian") {
    return recommendation.suitableFor.includes("vegan");
  }
  if (diet === "pescatarian") {
    return (
      recommendation.suitableFor.includes("vegetarian") ||
      recommendation.suitableFor.includes("vegan")
    );
  }
  return false;
}

function stableRotation(
  dateKey: string,
  mealId: MealId,
  choiceId: string,
): number {
  let hash = 2_166_136_261;
  for (const character of `${dateKey}:${mealId}:${choiceId}`) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16_777_619);
  }
  return (hash >>> 0) % 7;
}

function feedbackReasons(
  choiceId: string,
  history: ReadonlyArray<HistoryEntry>,
): RecommendationReason[] {
  const reasons: RecommendationReason[] = [];
  const relevant = [...history]
    .filter((entry) => entry.choiceId === choiceId)
    .sort((left, right) => Date.parse(right.at) - Date.parse(left.at));

  const acceptedCount = relevant.filter(
    (entry) => entry.action === "accepted",
  ).length;
  const skippedCount = relevant.filter(
    (entry) => entry.action === "skipped",
  ).length;
  const dislikedCount = relevant.filter(
    (entry) => entry.action === "disliked",
  ).length;

  if (acceptedCount > 0) {
    reasons.push({
      code: "accepted_before",
      points: Math.min(18, acceptedCount * 6),
    });
  }
  if (skippedCount > 0) {
    reasons.push({
      code: "skipped_before",
      points: -Math.min(30, skippedCount * 12),
    });
  }
  if (dislikedCount > 0) {
    reasons.push({
      code: "disliked_before",
      points: -Math.min(120, dislikedCount * 72),
    });
  }

  const recentChoices = [...history]
    .filter((entry) => entry.choiceId)
    .sort((left, right) => Date.parse(right.at) - Date.parse(left.at))
    .slice(0, 4);
  const recentIndex = recentChoices.findIndex(
    (entry) => entry.choiceId === choiceId,
  );
  if (recentIndex >= 0) {
    reasons.push({
      code: "recent_repeat",
      points: -Math.max(8, 26 - recentIndex * 6),
    });
  }
  return reasons;
}

function scoreRecommendation(
  recommendation: LocalRecommendation,
  input: RecommendationInput,
  dietCount: number,
  preferences: ReadonlyArray<SoftPreference>,
): ScoredRecommendation {
  const reasons: RecommendationReason[] = [
    { code: "catalog_baseline", points: 50 },
  ];
  const moodIndex = recommendation.moods.indexOf(input.mood);
  if (moodIndex === 0) {
    reasons.push({ code: "strong_mood_match", points: 24 });
  } else if (moodIndex > 0) {
    reasons.push({ code: "mood_match", points: 14 });
  }

  const budgetDifference =
    budgetRank[input.settings.budget] - budgetRank[recommendation.cost];
  reasons.push({ code: "budget_fit", points: budgetDifference === 0 ? 10 : 6 });
  if (dietCount > 0) {
    reasons.push({ code: "dietary_fit", points: Math.min(12, dietCount * 4) });
  }
  const preferenceMatches = preferences.filter((preference) =>
    recommendation.traits.includes(preference),
  ).length;
  const preferenceMisses = preferences.length - preferenceMatches;
  if (preferenceMatches > 0) {
    reasons.push({
      code: "preference_match",
      points: preferenceMatches * 9,
    });
  }
  if (preferenceMisses > 0) {
    reasons.push({
      code: "preference_miss",
      points: preferenceMisses * -4,
    });
  }
  reasons.push(...feedbackReasons(recommendation.id, input.history));
  reasons.push({
    code: "daily_rotation",
    points: stableRotation(input.dateKey, input.mealId, recommendation.id),
  });

  return {
    ...recommendation,
    score: reasons.reduce((total, reason) => total + reason.points, 0),
    reasons,
  };
}

export function recommendMeals(input: RecommendationInput): RecommendationSet {
  const diets = hardDiets(input.settings);
  const preferences = softPreferences(input.settings);
  const avoids = avoidTokens(input.settings);
  const excluded: ExcludedRecommendation[] = [];
  const candidates: ScoredRecommendation[] = [];

  for (const recommendation of RECOMMENDATION_CATALOG) {
    if (!recommendation.mealIds.includes(input.mealId)) {
      continue;
    }
    if (budgetRank[recommendation.cost] > budgetRank[input.settings.budget]) {
      excluded.push({ id: recommendation.id, reason: "budget" });
      continue;
    }
    if (diets.some((diet) => !supportsDiet(recommendation, diet))) {
      excluded.push({ id: recommendation.id, reason: "dietary" });
      continue;
    }
    if (conflictsWithAvoids(recommendation, avoids)) {
      excluded.push({ id: recommendation.id, reason: "avoid" });
      continue;
    }
    candidates.push(
      scoreRecommendation(recommendation, input, diets.length, preferences),
    );
  }

  candidates.sort(
    (left, right) =>
      right.score - left.score || left.id.localeCompare(right.id),
  );
  const limit = Math.max(1, Math.min(input.limit ?? 3, 6));
  return {
    ...(candidates[0] ? { primary: candidates[0] } : {}),
    alternatives: candidates.slice(1, limit),
    excluded,
  };
}
