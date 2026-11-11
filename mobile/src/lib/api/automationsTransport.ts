import type { RequestOptions } from "./authenticatedTransport";
import {
  automationIdSchema,
  automationListSchema,
  automationRunsPath,
  automationRunsSchema,
  automationSchema,
  automationSettingsSchema,
  type AutomationRunsCursor,
} from "../../features/automations/automationContracts";

type JsonRequest = (path: string, options?: RequestOptions) => Promise<unknown>;

/** Native automations are read-only. Keep every exposed operation GET-only. */
export function createAutomationsApi(request: JsonRequest) {
  return {
    async list(signal?: AbortSignal, orgId?: string) {
      const suffix = orgId
        ? `?${new URLSearchParams({ org_id: automationIdSchema.parse(orgId) })}`
        : "";
      return automationListSchema.parse(
        await request(`/triggers${suffix}`, { method: "GET", signal }),
      );
    },
    async detail(id: string, signal?: AbortSignal) {
      const row = automationSchema.parse(
        await request(`/triggers/${automationIdSchema.parse(id)}`, {
          method: "GET",
          signal,
        }),
      );
      if (row.id !== id)
        throw new Error(
          "The server returned a different automation. Please reload.",
        );
      return row;
    },
    async runs(
      id: string,
      cursor?: AutomationRunsCursor,
      signal?: AbortSignal,
    ) {
      return automationRunsSchema.parse(
        await request(automationRunsPath(id, cursor), {
          method: "GET",
          signal,
        }),
      );
    },
    async settings(signal?: AbortSignal) {
      return automationSettingsSchema.parse(
        await request("/assistant/nyxagent/settings", {
          method: "GET",
          signal,
        }),
      );
    },
  };
}
