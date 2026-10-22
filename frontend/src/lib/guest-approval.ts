import type { GuestApprovalContext } from "@/types/approvals";

export function guestApprovalLabel(guest: GuestApprovalContext): string {
  return `Guest request: ${guest.sender_name} via ${guest.platform} in ${guest.chat_title} · ${guest.agent_name}`;
}
