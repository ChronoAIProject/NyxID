import type { IssueGrantResponse } from "@/schemas/billing-credits";

export function rolloutWarningMessage(
  recipients: IssueGrantResponse["recipients"],
): string | null {
  const count = recipients.filter(
    (recipient) => !recipient.recipient_billing_enabled,
  ).length;
  if (count === 0) return null;
  return `${String(count)} ${count === 1 ? "recipient is" : "recipients are"} not in the billing rollout - ${count === 1 ? "user cannot" : "users cannot"} see billing yet.`;
}
