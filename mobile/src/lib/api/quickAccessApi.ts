import { localizedError } from "../../i18n/localizedError";
import { requestJson } from "./http";
import {
  QUICK_ACCESS_MAX_ITEMS,
  quickAccessItemSchema,
  quickAccessResponseSchema,
  type QuickAccessItem,
  type QuickAccessState,
} from "./quickAccessSchema";

const PATH = "/users/me/quick-access";

function parseState(payload: unknown): QuickAccessState {
  const result = quickAccessResponseSchema.safeParse(payload);
  if (!result.success)
    throw localizedError("NyxID returned an unexpected response. Please try again.");
  return result.data;
}

/** The Duo's quick-access rail, saved to the account (synced across devices). */
export const quickAccessApi = {
  async get(signal?: AbortSignal) {
    return parseState(await requestJson<unknown>(PATH, { signal }));
  },
  async save(items: readonly QuickAccessItem[], authorityGuard?: () => void) {
    // The same limits the server enforces, checked before sending.
    const body = {
      items: quickAccessItemSchema
        .array()
        .max(QUICK_ACCESS_MAX_ITEMS)
        .parse(items)
        .map(({ kind, id }) => ({ kind, id })),
    };
    return parseState(
      await requestJson<unknown>(PATH, { method: "PUT", body, authorityGuard }),
    );
  },
  async reset(authorityGuard?: () => void) {
    return parseState(
      await requestJson<unknown>(PATH, { method: "DELETE", authorityGuard }),
    );
  },
};
