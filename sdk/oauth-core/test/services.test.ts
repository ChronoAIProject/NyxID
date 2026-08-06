import { describe, expect, it, vi } from "vitest";

import {
  ConnectLinkExpiredError,
  ConnectLinkTimeoutError,
  NyxServicesClient,
  type ConnectLinkResponse,
  type TriggerResponse,
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
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(response));
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

    await expect(
      client.connectLinks.waitForCompletion("link-1"),
    ).rejects.toBeInstanceOf(ConnectLinkExpiredError);
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

function trigger(): TriggerResponse {
  return {
    id: "trigger-1",
    user_id: "user-1",
    label: "Repository activity",
    user_service_id: null,
    status: "active",
    verification: { mode: "token", location: "bearer" },
    delivery: { type: "notification" },
    inbound_url: "https://api.example/api/v1/webhooks/triggers/trigger-1",
    created_at: "2026-08-06T09:30:00.123+00:00",
    updated_at: "2026-08-06T09:30:00.123+00:00",
  };
}

describe("NyxServicesClient triggers", () => {
  it("creates triggers with exact backend tagged unions and field names", async () => {
    const created = {
      trigger: trigger(),
      secret: "nyx_trg_once",
      delivery_signing_secret: null,
    };
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValue(jsonResponse(created));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example/",
      auth: { apiKey: "nyxid_ag_test" },
      fetchFn,
    });

    await expect(
      client.triggers.create({
        label: "Repository activity",
        userServiceId: "service-1",
        targetOrgId: "org-1",
        verification: {
          mode: "hmac_sha256",
          header_name: "X-Hub-Signature-256",
        },
        delivery: {
          type: "webhook",
          url: "https://receiver.example/events",
        },
      }),
    ).resolves.toEqual(created);

    const [url, init] = fetchFn.mock.calls[0];
    expect(url).toBe("https://api.example/api/v1/triggers");
    expect(init?.method).toBe("POST");
    expect(new Headers(init?.headers).get("Authorization")).toBe(
      "Bearer nyxid_ag_test",
    );
    expect(JSON.parse(String(init?.body))).toEqual({
      label: "Repository activity",
      user_service_id: "service-1",
      target_org_id: "org-1",
      verification: {
        mode: "hmac_sha256",
        header_name: "X-Hub-Signature-256",
      },
      delivery: {
        type: "webhook",
        url: "https://receiver.example/events",
      },
    });
  });

  it("lists, gets, updates, rotates, and deletes using encoded routes", async () => {
    const item = trigger();
    const fetchFn = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse({ triggers: [item] }))
      .mockResolvedValueOnce(jsonResponse(item))
      .mockResolvedValueOnce(
        jsonResponse({
          trigger: { ...item, status: "disabled" },
          delivery_signing_secret: null,
        }),
      )
      .mockResolvedValueOnce(
        jsonResponse({ trigger: item, secret: "nyx_trg_rotated" }),
      )
      .mockResolvedValueOnce(jsonResponse({ message: "Trigger deleted" }));
    const client = new NyxServicesClient({
      baseUrl: "https://api.example",
      auth: { accessToken: "oauth-token" },
      fetchFn,
    });

    await expect(client.triggers.list()).resolves.toEqual({ triggers: [item] });
    await expect(client.triggers.get("trigger/1")).resolves.toEqual(item);
    await expect(
      client.triggers.update("trigger/1", { status: "disabled" }),
    ).resolves.toMatchObject({ trigger: { status: "disabled" } });
    await expect(
      client.triggers.rotateSecret("trigger/1"),
    ).resolves.toMatchObject({
      secret: "nyx_trg_rotated",
    });
    await expect(client.triggers.delete("trigger/1")).resolves.toEqual({
      message: "Trigger deleted",
    });

    expect(fetchFn.mock.calls.map(([url]) => url)).toEqual([
      "https://api.example/api/v1/triggers",
      "https://api.example/api/v1/triggers/trigger%2F1",
      "https://api.example/api/v1/triggers/trigger%2F1",
      "https://api.example/api/v1/triggers/trigger%2F1/rotate-secret",
      "https://api.example/api/v1/triggers/trigger%2F1",
    ]);
    expect(fetchFn.mock.calls[2][1]?.method).toBe("PATCH");
    expect(JSON.parse(String(fetchFn.mock.calls[2][1]?.body))).toEqual({
      status: "disabled",
    });
    expect(fetchFn.mock.calls[4][1]?.method).toBe("DELETE");
  });
});
