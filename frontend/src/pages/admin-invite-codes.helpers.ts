import type { InviteCode } from "@/types/admin";

export interface RedemptionRow {
  readonly id: string;
  readonly code: string;
  readonly codeId: string;
  readonly note: string | null;
  readonly userId: string;
  readonly userEmail: string | null;
  readonly userDisplayName: string | null;
  readonly usedAt: string;
}

/**
 * Flattens multiple invite codes and their usages into a flat array of redemption rows
 * sorted by the usedAt timestamp in descending order.
 */
export function flattenRedemptions(inviteCodes: readonly InviteCode[]): RedemptionRow[] {
  const rows: RedemptionRow[] = [];
  for (const code of inviteCodes) {
    for (const usage of code.usages) {
      rows.push({
        id: `${code.id}-${usage.user_id}-${usage.used_at}`,
        code: code.code,
        codeId: code.id,
        note: code.note,
        userId: usage.user_id,
        userEmail: usage.user_email,
        userDisplayName: usage.user_display_name,
        usedAt: usage.used_at,
      });
    }
  }
  return rows.sort((a, b) => b.usedAt.localeCompare(a.usedAt));
}
