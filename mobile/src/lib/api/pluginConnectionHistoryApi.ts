import { requestJson } from "./http";
import {
  parseConnectionHistory,
  parseHistoryActions,
} from "../../features/plugins/pluginConnectionHistoryTypes";

export const pluginConnectionHistoryApi = {
  async page(
    serviceId: string,
    cursor: string | null,
    actions: readonly string[],
    signal?: AbortSignal,
  ) {
    const params = new URLSearchParams({ limit: "20" });
    if (cursor) params.set("cursor", cursor);
    if (actions.length) params.set("actions", actions.join(","));
    return parseConnectionHistory(
      await requestJson(
        `/keys/${encodeURIComponent(serviceId)}/history?${params}`,
        { signal },
      ),
      serviceId,
      cursor,
    );
  },
  async actions(signal?: AbortSignal) {
    const items: ReturnType<typeof parseHistoryActions>["items"] = [];
    let offset = 0;
    let version: string | undefined;
    for (let pageCount = 0; pageCount < 12; pageCount++) {
      const page = parseHistoryActions(
        await requestJson(
          `/options/service-history-action?offset=${offset}&limit=100`,
          { signal },
        ),
        offset,
        version,
      );
      items.push(...page.items);
      if (page.next_offset === null) return items;
      offset = page.next_offset;
      version = page.version;
    }
    throw new Error("History filters exceed the supported page limit.");
  },
};
