import type { NyxAgentChannelAgent } from "@/schemas/assistant-nyxagent";
import { translateUi } from "@/i18n/use-ui-translation";

/** Use local structured copy; server explanations are not localized. */
export function botAudienceDescription(channel: NyxAgentChannelAgent): string {
  return channel.org_specialist
    ? translateUi(
        "Guests are people who have not verified an organization member account on this bot. The organization pays for their messages and tool use. Verified members keep their own permissions and billing. Only organization admins can change guest access.",
      )
    : translateUi(
        "Guests use your billing. A specialist's per-service guest access controls which services they can use. NyxBot does not use your services for guests.",
      );
}
