import { useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { DecisionCardActionTarget } from "./decision-card-presentation";

/** A pinned card's existing controls live in the stack's fixed footer. */
export function DecisionCardActions({ children }: { readonly children: ReactNode }) {
  const target = useContext(DecisionCardActionTarget);
  if (!children) return null;
  return target ? createPortal(children, target) : children;
}
