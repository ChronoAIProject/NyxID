import type { ComponentProps } from "react";
import { TEXT_STYLES } from "@/lib/typography";
import { cn } from "@/lib/utils";

/** Approval fields retain the same roles in cards, grouped rows and tables. */
export function ApprovalIdentity({ className, ...props }: ComponentProps<"span">) {
  return <span data-approval-field="identity" className={cn(TEXT_STYLES.rowTitle, "text-foreground", className)} {...props} />;
}

export function ApprovalAction({ className, ...props }: ComponentProps<"span">) {
  return <span data-approval-field="action" className={cn(TEXT_STYLES.body, "text-muted-foreground", className)} {...props} />;
}
