import type { ApprovalMode, ChallengeItem } from "./api/types";

type Guest = NonNullable<ChallengeItem["guest"]>;

export function guestApprovalMode(
  guest: Guest | null | undefined,
  mode: ApprovalMode,
): ApprovalMode {
  return guest ? "per_request" : mode;
}

export function guestApprovalLabel(guest: Guest): string {
  return `Guest request: ${guest.sender_name} via ${guest.platform} in ${guest.chat_title} · ${guest.agent_name}`;
}
