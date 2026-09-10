import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelHostedConnectLink,
  compactCompleteConnectLinkInput,
  connectLinkStorageKey,
  createConnectLink,
} from "@/hooks/use-connect-links";
import { api } from "@/lib/api-client";

vi.mock("@/lib/api-client", () => ({
  api: { post: vi.fn() },
}));

const post = vi.mocked(api.post);

beforeEach(() => post.mockReset());

describe("connect link OAuth storage", () => {
  it("sends only a page selector and validates the resolved destination response", async () => {
    post.mockResolvedValue({
      id: "65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
      connect_url: "https://app.example/connect/secret",
      expires_at: "2026-09-10T12:00:00Z",
      callback_url: "http://127.0.0.1:3003/temp",
    });
    await expect(
      createConnectLink({
        service_slug: "api-google",
        return_page: "local-poc",
      }),
    ).resolves.toMatchObject({ callback_url: "http://127.0.0.1:3003/temp" });
    expect(post).toHaveBeenCalledWith("/connect-links", {
      service_slug: "api-google",
      return_page: "local-poc",
    });
  });

  it("rejects a URL supplied as a page selector before any request", async () => {
    await expect(
      createConnectLink({
        service_slug: "api-google",
        return_page: "https://evil.example",
      }),
    ).rejects.toThrow();
    expect(post).not.toHaveBeenCalled();
  });

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

  it("cancels a hosted request with the raw token in the request body", async () => {
    post.mockResolvedValue({
      id: "65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
      status: "cancelled",
      service_name: "GitHub",
      service_slug: "github",
      expires_at: "2026-08-05T10:15:00Z",
      callback_url:
        "desktop-app://connect/return?status=cancelled&connect_link_id=65dd8fe8-9ee8-4c89-af1e-b283a17bcf37",
    });

    await expect(
      cancelHostedConnectLink("nyx_clk_secret"),
    ).resolves.toMatchObject({
      status: "cancelled",
    });
    expect(post).toHaveBeenCalledWith("/connect-links/cancel", {
      token: "nyx_clk_secret",
    });
  });
});
