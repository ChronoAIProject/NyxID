import { describe, expect, it, vi } from "vitest";

import {
  ConnectLinkExpiredError,
  ConnectLinkTimeoutError,
  NyxServicesClient,
  type ConnectLinkResponse,
} from "../src/index.js";

function link(
  status: ConnectLinkResponse["status"],
  connectedService?: ConnectLinkResponse["connected_service"],
): ConnectLinkResponse {
  return {
    id: "link-1",
    status,
    service_name: "GitHub",
    service_slug: "github",
    expires_at: "2026-08-05T12:00:00Z",
    connected_service: connectedService,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

describe("NyxServicesClient connect links", () => {
  it("creates a connect link with the backend wire field names", async () => {
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(
      jsonResponse({
        id: "link-1",
        connect_url: "https://app.example/connect/secret",
        expires_at: "2026-08-05T12:00:00Z",
      }),
    );
    const client = new NyxServicesClient({
      baseUrl: "https://api.example/",
      auth: { apiKey: "nyxid_ag_test" },
      fetchFn,
    });

    const created = await client.connectLinks.create({
      serviceSlug: "github",
      label: "work",
      callbackUrl: "https://agent.example/return",
      expiresIn: 600,
    });

    expect(created.id).toBe("link-1");
    const [url, init] = fetchFn.mock.calls[0];
    expect(url).toBe("https://api.example/api/v1/connect-links");
    expect(init?.method).toBe("POST");
    expect(new Headers(init?.headers).get("Authorization")).toBe(
      "Bearer nyxid_ag_test",
    );
    expect(JSON.parse(String(init?.body))).toEqual({
      service_slug: "github",
      label: "work",
      callback_url: "https://agent.example/return",
      expires_in: 600,
    });
  });

  it("waits through pending and resolves with the connected service", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(link("pending")))
      .mockResolvedValueOnce(
        jsonResponse(link("completed", { id: "service-1", slug: "github" })),
      );
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { accessToken: "oauth-access-token" },
      fetchFn,
    });

    await expect(
      client.connectLinks.waitForCompletion("link-1", {
        timeoutMs: 100,
        intervalMs: 1,
      }),
    ).resolves.toEqual({ id: "service-1", slug: "github" });
    expect(fetchFn).toHaveBeenCalledTimes(2);
    expect(
      new Headers(fetchFn.mock.calls[0][1]?.headers).get("Authorization"),
    ).toBe("Bearer oauth-access-token");
  });

  it("preserves app identity and provider-decline status fields", async () => {
    const response = {
      ...link("pending"),
      requesting_app_id: "desktop-client",
      requesting_app_name: "Desktop App",
      last_error: "provider_access_denied",
      last_error_at: "2026-08-05T11:00:00Z",
    };
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(jsonResponse(response));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { accessToken: "oauth-access-token" },
      fetchFn,
    });

    await expect(client.connectLinks.get("link-1")).resolves.toEqual(response);
  });

  it("rejects an expired link with a typed error", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(link("expired")));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { apiKey: "nyxid_ag_test" },
      fetchFn,
    });

    await expect(client.connectLinks.waitForCompletion("link-1")).rejects.toBeInstanceOf(
      ConnectLinkExpiredError,
    );
  });

  it("rejects pending links after the configured timeout", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockImplementation(async () => jsonResponse(link("pending")));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { apiKey: "nyxid_ag_test" },
      fetchFn,
    });

    await expect(
      client.connectLinks.waitForCompletion("link-1", {
        timeoutMs: 5,
        intervalMs: 2,
      }),
    ).rejects.toBeInstanceOf(ConnectLinkTimeoutError);
    expect(fetchFn).toHaveBeenCalled();
  });

  it("cancels a connect link", async () => {
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(link("cancelled")));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { apiKey: "nyxid_ag_test" },
      fetchFn,
    });

    await expect(client.connectLinks.cancel("link/1")).resolves.toMatchObject({
      status: "cancelled",
    });
    expect(fetchFn.mock.calls[0][0]).toBe(
      "https://api.example/api/v1/connect-links/link%2F1/cancel",
    );
    expect(fetchFn.mock.calls[0][1]?.method).toBe("POST");
  });
});

describe("NyxServicesClient service requests", () => {
  it("constructs the proxy URL and preserves the raw response", async () => {
    const upstream = new Response("streamable", { status: 202 });
    const fetchFn = vi.fn<typeof fetch>().mockResolvedValue(upstream);
    const client = new NyxServicesClient({
      baseUrl: "https://api.example/",
      auth: { accessToken: "oauth-access-token" },
      fetchFn,
    });

    const response = await client.services.request("github/work", "/repos", {
      method: "POST",
      headers: {
        Authorization: "Bearer caller-value",
        "Content-Type": "application/json",
      },
      body: '{"name":"nyxid"}',
      query: { page: 2, tag: ["one", "two"], ignored: null },
    });

    expect(response).toBe(upstream);
    const [url, init] = fetchFn.mock.calls[0];
    expect(String(url)).toBe(
      "https://api.example/api/v1/proxy/s/github%2Fwork/repos?page=2&tag=one&tag=two",
    );
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe('{"name":"nyxid"}');
    expect(new Headers(init?.headers).get("Authorization")).toBe(
      "Bearer oauth-access-token",
    );
  });
});
