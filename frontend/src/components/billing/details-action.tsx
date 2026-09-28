import { ChevronDown } from "lucide-react";

/** The "Details ▾" affordance pinned top-right of every benefit summary. */
export function DetailsAction() {
  return (
    <span className="compact-benefit-action">
      Details <ChevronDown size={13} className="disclosure-arrow" />
    </span>
  );
}
