import type { NyxAgentChannelAgent } from "@/schemas/assistant-nyxagent";

/** Older servers omit the explanation; current UI and chat use the server copy. */
export function botAudienceDescription(channel: NyxAgentChannelAgent): string {
  return (
    channel.guest_access_description ??
    (channel.org_specialist
      ? "Verified organization members use their own permissions and billing. Guests need guest access and a daily budget paid by the organization. Only organization admins can change guest access."
      : "Guests use your billing. A specialist's per-service guest access controls which services they can use. NyxBot does not use your services for guests.")
  );
}
