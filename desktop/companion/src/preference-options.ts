import type { Budget } from "./domain";

export const DIET_OPTIONS = [
  { id: "light", label: "偏清淡" },
  { id: "high-protein", label: "高蛋白" },
  { id: "vegetarian", label: "素食" },
  { id: "low-oil", label: "少油" },
  { id: "mild", label: "不太辣" },
] as const;

export const BUDGET_OPTIONS: ReadonlyArray<{ id: Budget; label: string }> = [
  { id: "low", label: "省一点" },
  { id: "everyday", label: "日常" },
  { id: "flexible", label: "随心" },
];
