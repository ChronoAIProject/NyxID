import { localizedError } from "../../i18n/localizedError";
import {
  connectLinkPreviewSchema,
  connectLinkStatusResponseSchema,
  completeConnectLinkResponseSchema,
  createConnectLinkResponseSchema,
  type CompleteConnectLinkInput,
} from "../../features/assistant/connectLinkContracts";
import { requestJson } from "./http";

export const connectLinksApi = {
  /** Mint a hosted connect link for a catalog service on the signed-in account. */
  async create(
    serviceSlug: string,
    signal?: AbortSignal,
    scopes?: readonly string[],
    marketplace?: { personal: boolean },
  ) {
    return createConnectLinkResponseSchema.parse(
      await requestJson("/connect-links", {
        method: "POST",
        body: {
          service_slug: serviceSlug,
          ...(!marketplace ? { requested_by: "NyxBot chat" } : {}),
          ...(marketplace?.personal ? { use_platform_key: false, assistant_personal: true } : {}),
          ...(scopes?.length ? { scopes } : {}),
        },
        signal,
      }),
    );
  },
  async preview(token: string, signal?: AbortSignal) {
    return connectLinkPreviewSchema.parse(
      await requestJson("/connect-links/preview", {
        method: "POST",
        body: { token },
        signal,
      }),
    );
  },
  async complete(
    token: string,
    values: CompleteConnectLinkInput,
    signal?: AbortSignal,
  ) {
    const compact = Object.fromEntries(
      Object.entries(values).filter(
        ([, value]) =>
          typeof value === "boolean" ||
          (typeof value === "string" && value.trim().length > 0),
      ),
    );
    return completeConnectLinkResponseSchema.parse(
      await requestJson("/connect-links/complete", {
        method: "POST",
        body: { token, ...compact },
        signal,
      }),
    );
  },
  async cancel(token: string, signal?: AbortSignal) {
    return connectLinkStatusResponseSchema.parse(
      await requestJson("/connect-links/cancel", {
        method: "POST",
        body: { token },
        signal,
      }),
    );
  },
  async status(id: string, signal?: AbortSignal) {
    const result = connectLinkStatusResponseSchema.parse(
      await requestJson(`/connect-links/${encodeURIComponent(id)}`, { signal }),
    );
    if (result.id !== id)
      throw localizedError("The server returned a different connection request. Please reload.");
    return result;
  },
};
