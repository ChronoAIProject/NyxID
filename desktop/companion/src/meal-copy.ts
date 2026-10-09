import type { RecommendationMood, ScoredRecommendation } from "./domain";

interface MealCopy {
  readonly name: string;
  readonly detail: string;
}

const MEAL_COPY: Readonly<Record<string, MealCopy>> = {
  "banana-oat-bowl": {
    name: "香蕉燕麦碗",
    detail: "香蕉、燕麦奶和一点肉桂",
  },
  "savory-egg-toast": {
    name: "菠菜鸡蛋吐司",
    detail: "烤吐司配嫩蛋和菠菜",
  },
  "berry-yogurt-crunch": {
    name: "莓果酸奶脆脆碗",
    detail: "酸奶、莓果和少量坚果",
  },
  "avocado-breakfast-tacos": {
    name: "牛油果早餐塔可",
    detail: "玉米饼、豆子和新鲜番茄",
  },
  "lemon-lentil-soup": {
    name: "柠檬扁豆汤",
    detail: "扁豆、胡萝卜和清爽柠檬",
  },
  "sesame-tofu-rice": {
    name: "芝麻豆腐饭",
    detail: "煎豆腐、西兰花和米饭",
  },
  "herbed-chicken-rice": {
    name: "香草鸡肉饭",
    detail: "鸡肉、黄瓜和香草米饭",
  },
  "roasted-vegetable-pasta": {
    name: "烤蔬菜意面",
    detail: "番茄、西葫芦和烤香蔬菜",
  },
  "salmon-greens-plate": {
    name: "柠檬三文鱼蔬菜盘",
    detail: "三文鱼、绿叶菜和柠檬",
  },
  "mushroom-dumpling-bowl": {
    name: "菌菇饺子汤碗",
    detail: "菌菇饺子、青菜和芝麻",
  },
  "steak-sweet-potato": {
    name: "牛排烤红薯",
    detail: "牛排、烤红薯和绿叶菜",
  },
  "chickpea-flatbread": {
    name: "鹰嘴豆烤饼",
    detail: "鹰嘴豆、番茄和香草烤饼",
  },
};

const MOOD_LABELS: Record<RecommendationMood, string> = {
  light: "清淡",
  balanced: "正常吃",
  treat: "犒劳一下",
};

const COST_LABELS = {
  low: "省钱档",
  everyday: "日常预算",
  flexible: "随心预算",
} as const;

export const MEAL_LABELS = {
  breakfast: "早餐",
  lunch: "午餐",
  dinner: "晚餐",
} as const;

export interface RecommendationCopy {
  readonly name: string;
  readonly detail: string;
  readonly reason: string;
}

function strongestReason(
  recommendation: ScoredRecommendation,
  mood: RecommendationMood,
): string {
  const codes = new Set(recommendation.reasons.map((reason) => reason.code));
  if (codes.has("disliked_before")) {
    return "你说过不喜欢，已经降低了推荐顺序";
  }
  if (codes.has("accepted_before")) {
    return "你之前选过它，这次也很合适";
  }
  if (codes.has("dietary_fit")) {
    return "符合你保存的饮食偏好";
  }
  if (codes.has("preference_match")) {
    return "更贴近你平时喜欢的口味";
  }
  if (codes.has("strong_mood_match")) {
    return `很贴合今天“${MOOD_LABELS[mood]}”的感觉`;
  }
  if (codes.has("budget_fit")) {
    return `落在你的${COST_LABELS[recommendation.cost]}`;
  }
  return "今天换个口味，避免总吃重复的";
}

export function recommendationCopy(
  recommendation: ScoredRecommendation,
  mood: RecommendationMood,
): RecommendationCopy {
  const copy = MEAL_COPY[recommendation.id] ?? {
    name: recommendation.id,
    detail: recommendation.signals.join("、"),
  };
  return {
    ...copy,
    detail: `${copy.detail} · 约 ${String(recommendation.prepMinutes)} 分钟`,
    reason: strongestReason(recommendation, mood),
  };
}
