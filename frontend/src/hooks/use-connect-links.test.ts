import { describe, expect, it } from "vitest";
import {
  compactCompleteConnectLinkInput,
  connectLinkStorageKey,
} from "@/hooks/use-connect-links";

describe("connect link OAuth storage", () => {
  it("namespaces the token by link id", () => {
    expect(connectLinkStorageKey("link-123")).toBe(
      "nyxid:connect-link:link-123",
    );
  });

  it("omits empty optional secrets without dropping a device polling state", () => {
    expect(
      compactCompleteConnectLinkInput({
        credential: "secret",
        endpoint_url: "",
        oauth_client_id: "",
        oauth_client_secret: "",
        device_state: "device-state",
      }),
    ).toEqual({ credential: "secret", device_state: "device-state" });
  });
});
