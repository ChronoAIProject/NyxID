import {
  connectLinkPreviewSchema,
  connectLinkStatusResponseSchema,
  completeConnectLinkResponseSchema,
  type CompleteConnectLinkInput,
} from "../../features/assistant/connectLinkContracts";
import { requestJson } from "./http";

export const connectLinksApi = {
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
      throw new Error(
        "The server returned a different connection request. Please reload.",
      );
    return result;
  },
};
