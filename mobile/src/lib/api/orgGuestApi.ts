import { requestJson } from "./http";
import type { RequestOptions } from "./authenticatedTransport";
import {
  orgGuestReviewSchema,
  orgGuestSettingsSchema,
  orgGuestSwitchPayload,
} from "../../features/agents/orgGuestContracts";
const channelPath = (id: string) =>
  `/assistant/nyxagent/channels/${encodeURIComponent(id)}/org-specialist`;
const agentPath = (id: string) =>
  `/assistant/nyxagent/agents/${encodeURIComponent(id)}/org-guest-grants`;
export const orgGuestApi = {
  async settings(id: string, signal?: AbortSignal) {
    return orgGuestSettingsSchema.parse(
      await requestJson(channelPath(id), { signal }),
    );
  },
  async toggle(
    id: string,
    enabled: boolean,
    revision: string,
    authorityGuard: RequestOptions["authorityGuard"],
  ) {
    return orgGuestSettingsSchema.parse(
      await requestJson(channelPath(id), {
        method: "PUT",
        body: orgGuestSwitchPayload(enabled, revision),
        authorityGuard,
      }),
    );
  },
  async preview(id: string, signal?: AbortSignal) {
    return orgGuestReviewSchema.parse(
      await requestJson(agentPath(id), { signal }),
    );
  },
  async adopt(
    id: string,
    review: string,
    authorityGuard: RequestOptions["authorityGuard"],
  ) {
    return orgGuestReviewSchema.parse(
      await requestJson(agentPath(id), {
        method: "POST",
        body: { review },
        authorityGuard,
      }),
    );
  },
};
