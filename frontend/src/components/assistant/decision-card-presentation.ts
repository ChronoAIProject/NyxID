import { createContext, useContext } from "react";

export const DecisionCardPresentation = createContext<"transcript" | "pinned">("transcript");
export const DecisionCardActionTarget = createContext<HTMLDivElement | null>(null);
export const DecisionCardInteractionBlocked = createContext(false);
export const DecisionCardActive = createContext(true);

export function usePinnedDecisionCard(): boolean {
  return useContext(DecisionCardPresentation) === "pinned";
}
