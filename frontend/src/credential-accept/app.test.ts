import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bootCredentialAcceptPage } from "./app";

const cryptoMock = vi.hoisted(() => ({
  capturedPlaintexts: [] as Uint8Array[],
  buildRciContext: vi.fn(() => "rci-context"),
  encrypt: vi.fn((plaintext: Uint8Array) => {
    cryptoMock.capturedPlaintexts.push(plaintext);
    return {
      version: "v1",
      admin_pubkey: "admin-public",
      nonce: "nonce-value",
      ciphertext: "cipher-value",
    };
  }),
}));

vi.mock("@/lib/crypto", () => ({
  buildRciContext: cryptoMock.buildRciContext,
  encrypt: cryptoMock.encrypt,
}));

const scriptBytes = new TextEncoder().encode("globalThis.__nyxidAccept = true;");
const nowMs = Date.parse("2026-06-05T00:00:00.000Z");

interface RuntimeConfigFixture {
  readonly api_base_url: string;
  readonly release_integrity: {
    readonly enabled: boolean;
    readonly manifest_url: string | null;
    readonly verification_ttl_secs: number;
  };
}

class MemoryStorage implements Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

function runtimeConfig(overrides: Partial<RuntimeConfigFixture["release_integrity"]> = {}): RuntimeConfigFixture {
  return {
    api_base_url: "http://localhost",
    release_integrity: {
      enabled: true,
      manifest_url: "https://release.example.test/releases.json",
      verification_ttl_secs: 1800,
      ...overrides,
    },
  };
}

function pubkeyResponse(optOut = false) {
  return {
    pending_id: "pending-1",
    node_id: "node-1",
    service_slug: "openclaw",
    version: "v1",
    node_pubkey: "node-public",
    remote_state: "pubkey_posted",
    integrity_verification_opt_out: optOut,
  };
}

function pendingListResponse() {
  return {
    pending_credentials: [
      {
        id: "pending-1",
        node_id: "node-1",
        service_slug: "openclaw",
        injection_method: "header",
        field_name: "X-API-Key",
        target_url: "https://gateway.example.test/v1",
        label: "Production",
        created_by_user_id: "user-1",
        owner_user_id: "user-1",
        created_at: "2026-06-05T00:00:00Z",
        expires_at: "2026-06-05T01:00:00Z",
        remote_state: "pubkey_posted",
        is_active: true,
      },
    ],
  };
}

function fanOutStatusResponse() {
  return {
    fanout_id: "fanout-1",
    fan_out_revision: 7,
    target_count: 2,
    service_slug: "openclaw",
    injection_method: "header",
    field_name: "X-API-Key",
    target_url: "https://gateway.example.test/v1",
    label: "Production",
    remote_state: "pubkey_posted",
    targets: [
      {
        node_id: "node-a",
        generation: 0,
        remote_state: "pubkey_posted",
        delivery_status: null,
        error_code: null,
        error_kind: null,
      },
      {
        node_id: "node-b",
        generation: 2,
        remote_state: "pubkey_posted",
        delivery_status: null,
        error_code: null,
        error_kind: null,
      },
    ],
  };
}

function fanOutPubkeysResponse(revision: number, retryReady = false) {
  return {
    fanout_id: "fanout-1",
    fan_out_revision: revision,
    target_count: 2,
    integrity_verification_opt_out: false,
    targets: [
      {
        node_id: "node-a",
        generation: 0,
        version: "v1",
        node_pubkey: "pubkey-a",
        remote_state: retryReady ? "consumed" : "pubkey_posted",
        error_code: null,
      },
      {
        node_id: "node-b",
        generation: retryReady ? 3 : 2,
        version: "v1",
        node_pubkey: "pubkey-b",
        remote_state: "pubkey_posted",
        error_code: null,
      },
    ],
  };
}

function fanOutPartialResponse() {
  return {
    fanout_id: "fanout-1",
    fan_out_revision: 8,
    remote_state: "partial_decrypted",
    targets: [
      {
        node_id: "node-a",
        generation: 0,
        remote_state: "consumed",
        delivery_status: "sent",
        error_code: null,
        error_kind: null,
      },
      {
        node_id: "node-b",
        generation: 2,
        remote_state: "decrypt_failed",
        delivery_status: "sent",
        error_code: 8006,
        error_kind: "pending_credential_decrypt_failed",
      },
    ],
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function setupFanOutPage() {
  window.history.pushState(
    null,
    "",
    "/nodes/credentials/pending/fanout-1/fan-out/accept",
  );
  document.body.innerHTML = `
    <div id="credential-accept-root"></div>
  `;
  const root = document.getElementById("credential-accept-root");
  if (!root) throw new Error("root missing");
  const originalQuerySelectorAll = document.querySelectorAll.bind(document);
  vi.spyOn(document, "querySelectorAll").mockImplementation((selector: string) => {
    if (
      selector ===
      'script[data-nyx-integrity-role="credential_accept_script"][src]'
    ) {
      return [
        {
          src: new URL(
            "/credential-accept/assets/credential-accept-test.js",
            window.location.href,
          ).href,
        },
      ] as unknown as NodeListOf<Element>;
    }
    return originalQuerySelectorAll(selector);
  });

  const storage = new MemoryStorage();
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let pubkeysCalls = 0;
  let ciphertextCalls = 0;
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" || input instanceof URL ? String(input) : input.url;
    calls.push({ url, init });
    const parsed = new URL(url, window.location.href);
    const endpoint = `${parsed.pathname}${parsed.search}`;

    if (endpoint === "/credential-accept/assets/credential-accept-test.js") {
      return new Response(scriptBytes);
    }
    if (endpoint === "/api/v1/runtime-config") {
      return jsonResponse(runtimeConfig());
    }
    if (endpoint === "/api/v1/nodes/credentials/pending/fanout-1/fan-out") {
      return jsonResponse(fanOutStatusResponse());
    }
    if (endpoint === "/api/v1/nodes/credentials/pending/fanout-1/fan-out/pubkeys") {
      pubkeysCalls += 1;
      return jsonResponse(fanOutPubkeysResponse(pubkeysCalls >= 3 ? 9 : 7, pubkeysCalls >= 3));
    }
    if (endpoint === "/api/v1/nodes/credentials/pending/fanout-1/fan-out/ciphertexts") {
      ciphertextCalls += 1;
      if (ciphertextCalls === 1) {
        return jsonResponse(fanOutPartialResponse());
      }
      return jsonResponse({
        fanout_id: "fanout-1",
        fan_out_revision: 10,
        remote_state: "consumed",
        targets: [
          {
            node_id: "node-a",
            generation: 0,
            remote_state: "consumed",
            delivery_status: "sent",
            error_code: null,
            error_kind: null,
          },
          {
            node_id: "node-b",
            generation: 3,
            remote_state: "consumed",
            delivery_status: "sent",
            error_code: null,
            error_kind: null,
          },
        ],
      });
    }
    if (endpoint === "/api/v1/nodes/credentials/pending/fanout-1/fan-out/retry-failed") {
      return jsonResponse({
        fanout_id: "fanout-1",
        fan_out_revision: 9,
        target_count: 2,
        service_slug: "openclaw",
        injection_method: "header",
        field_name: "X-API-Key",
        target_url: "https://gateway.example.test/v1",
        label: "Production",
        remote_state: "partial_decrypted",
        targets: [
          {
            node_id: "node-a",
            generation: 0,
            remote_state: "consumed",
            delivery_status: null,
            error_code: null,
            error_kind: null,
          },
          {
            node_id: "node-b",
            generation: 3,
            remote_state: null,
            delivery_status: null,
            error_code: null,
            error_kind: null,
          },
        ],
      });
    }
    return jsonResponse({ message: `unexpected ${endpoint}`, error_code: 9999 }, 500);
  });

  bootCredentialAcceptPage(root, {
    fetch: fetchMock as unknown as typeof fetch,
    location: window.location,
    window,
    document,
    storage,
    now: () => nowMs,
    delay: async () => {},
  });

  return { calls, fetchMock, root, storage };
}

function setupPage(params: {
  readonly runtime?: RuntimeConfigFixture;
  readonly optOut?: boolean;
}) {
  window.history.pushState(
    null,
    "",
    "/nodes/node-1/credentials/pending/pending-1/accept",
  );
  document.body.innerHTML = `
    <div id="credential-accept-root"></div>
  `;
  const root = document.getElementById("credential-accept-root");
  if (!root) throw new Error("root missing");
  const originalQuerySelectorAll = document.querySelectorAll.bind(document);
  vi.spyOn(document, "querySelectorAll").mockImplementation((selector: string) => {
    if (
      selector ===
      'script[data-nyx-integrity-role="credential_accept_script"][src]'
    ) {
      return [
        {
          src: new URL(
            "/credential-accept/assets/credential-accept-test.js",
            window.location.href,
          ).href,
        },
      ] as unknown as NodeListOf<Element>;
    }
    return originalQuerySelectorAll(selector);
  });

  const storage = new MemoryStorage();
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" || input instanceof URL ? String(input) : input.url;
    calls.push({ url, init });
    const parsed = new URL(url, window.location.href);
    const endpoint = `${parsed.pathname}${parsed.search}`;

    if (endpoint === "/credential-accept/assets/credential-accept-test.js") {
      return new Response(scriptBytes);
    }
    if (endpoint === "/api/v1/runtime-config") {
      return jsonResponse(params.runtime ?? runtimeConfig());
    }
    if (endpoint === "/api/v1/nodes/node-1/credentials/pending/pending-1") {
      return jsonResponse(pubkeyResponse(Boolean(params.optOut)));
    }
    if (endpoint === "/api/v1/nodes/node-1/credentials/pending?include_history=true") {
      return jsonResponse(pendingListResponse());
    }
    if (endpoint === "/api/v1/nodes/node-1/credentials/pending/pending-1/ciphertext") {
      return jsonResponse({ delivery_status: "sent", remote_state: "consumed" });
    }
    return jsonResponse({ message: `unexpected ${endpoint}`, error_code: 9999 }, 500);
  });

  bootCredentialAcceptPage(root, {
    fetch: fetchMock as unknown as typeof fetch,
    location: window.location,
    window,
    document,
    storage,
    now: () => nowMs,
    delay: async () => {},
  });

  return { calls, fetchMock, root, storage };
}

function postBody(calls: Array<{ url: string; init?: RequestInit }>): Record<string, unknown> {
  const post = calls.find(
    (call) =>
      call.url.includes("/ciphertext") && call.init?.method === "POST",
  );
  expect(post).toBeDefined();
  return JSON.parse(String(post?.init?.body)) as Record<string, unknown>;
}

function postBodies(
  calls: Array<{ url: string; init?: RequestInit }>,
  path: string,
): Array<Record<string, unknown>> {
  return calls
    .filter((call) => call.url.includes(path) && call.init?.method === "POST")
    .map((call) => JSON.parse(String(call.init?.body)) as Record<string, unknown>);
}

beforeEach(() => {
  cryptoMock.capturedPlaintexts.length = 0;
  cryptoMock.buildRciContext.mockClear();
  cryptoMock.encrypt.mockClear();
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("standalone credential accept page", () => {
  it("shows the computed fingerprint and gates submit on out-of-band verification", async () => {
    setupPage({});

    await waitFor(() => {
      expect(screen.getByTestId("fingerprint")).toHaveTextContent(/^[0-9a-f]{12}$/);
    });
    expect(screen.getByRole("button", { name: "Accept" })).toBeDisabled();

    fireEvent.click(screen.getByRole("checkbox", { name: /verified the fingerprint/i }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Accept" })).not.toBeDisabled();
    });
  });

  it("fails closed when the manifest URL is unset and org policy has not opted out", async () => {
    const { calls } = setupPage({
      runtime: runtimeConfig({ enabled: false, manifest_url: null }),
    });

    await waitFor(() => {
      expect(
        screen.getByText(/manifest URL is not configured; submit is blocked/i),
      ).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "Accept" })).toBeDisabled();

    const input = screen.getByLabelText("Credential value") as HTMLInputElement;
    input.value = "secret-value-fixture";
    fireEvent.input(input);
    fireEvent.submit(input.closest("form")!);

    await waitFor(() => {
      expect(
        screen.getByText("Release integrity manifest URL is not configured."),
      ).toBeInTheDocument();
    });
    expect(calls.some((call) => call.url.includes("/ciphertext"))).toBe(false);
    expect(input.value).toBe("");
  });

  it("sends admin verification metadata without leaking plaintext and zeros plaintext bytes", async () => {
    const { calls, root, storage } = setupPage({});

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Accept" })).toBeDisabled();
    });
    fireEvent.click(screen.getByRole("checkbox", { name: /verified the fingerprint/i }));
    const input = screen.getByLabelText("Credential value") as HTMLInputElement;
    input.value = "secret-value-fixture";
    fireEvent.input(input);
    fireEvent.submit(input.closest("form")!);

    await waitFor(() => {
      expect(screen.getByText("Stored")).toBeInTheDocument();
    });

    const body = postBody(calls);
    expect(body.integrity_verification).toMatchObject({
      mode: "admin_verified",
      verified_at: "2026-06-05T00:00:00.000Z",
      manifest_url_configured: true,
    });
    expect(String(JSON.stringify(body))).not.toContain("secret-value-fixture");
    expect(input.value).toBe("");
    expect(root.textContent).not.toContain("secret-value-fixture");
    expect(Array.from(storage.values.values()).join("\n")).not.toContain(
      "secret-value-fixture",
    );
    expect(cryptoMock.capturedPlaintexts).toHaveLength(1);
    expect([...cryptoMock.capturedPlaintexts[0]!]).toEqual(
      Array(cryptoMock.capturedPlaintexts[0]!.length).fill(0),
    );
  });

  it("allows org policy opt-out and sends opt-out integrity metadata", async () => {
    const { calls } = setupPage({
      runtime: runtimeConfig({ enabled: false, manifest_url: null }),
      optOut: true,
    });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Accept" })).not.toBeDisabled();
    });
    expect(screen.getByRole("checkbox", { name: /verified the fingerprint/i }))
      .toBeDisabled();

    const input = screen.getByLabelText("Credential value") as HTMLInputElement;
    input.value = "org-secret-fixture";
    fireEvent.input(input);
    fireEvent.submit(input.closest("form")!);

    await waitFor(() => {
      expect(screen.getByText("Stored")).toBeInTheDocument();
    });

    const body = postBody(calls);
    expect(body.integrity_verification).toEqual({
      mode: "org_policy_opt_out",
      fingerprint_sha384_hex: null,
      verified_at: null,
      manifest_url_configured: false,
    });
    expect(JSON.stringify(body)).not.toContain("org-secret-fixture");
    expect([...cryptoMock.capturedPlaintexts[0]!]).toEqual(
      Array(cryptoMock.capturedPlaintexts[0]!.length).fill(0),
    );
  });

  it("retries failed fan-out targets with fresh scoped ciphertexts without leaking plaintext", async () => {
    const { calls, root, storage } = setupFanOutPage();
    const secret = "fanout-secret-fixture";

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Accept" })).toBeDisabled();
    });
    fireEvent.click(screen.getByRole("checkbox", { name: /verified the fingerprint/i }));
    const input = screen.getByLabelText("Credential value") as HTMLInputElement;
    input.value = secret;
    fireEvent.input(input);
    fireEvent.submit(input.closest("form")!);

    await waitFor(() => {
      expect(screen.getByText("Partially stored")).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "Retry failed" })).toBeInTheDocument();
    expect(screen.getByText("node-a")).toBeInTheDocument();
    expect(screen.getByText("node-b")).toBeInTheDocument();
    expect(screen.getByText("decrypt_failed")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Retry failed" }));

    await waitFor(() => {
      expect(screen.getByText("Stored")).toBeInTheDocument();
    });

    const retryBodies = postBodies(calls, "/fan-out/retry-failed");
    expect(retryBodies).toEqual([{ fan_out_revision: 8 }]);
    const retryCall = calls.find(
      (call) => call.url.includes("/fanout-1/fan-out/retry-failed"),
    );
    expect(retryCall).toBeDefined();

    const ciphertextBodies = postBodies(calls, "/fan-out/ciphertexts");
    expect(ciphertextBodies).toHaveLength(2);
    expect(ciphertextBodies[0]).toMatchObject({
      fan_out_revision: 7,
      items: [
        { node_id: "node-a", generation: 0 },
        { node_id: "node-b", generation: 2 },
      ],
    });
    expect(ciphertextBodies[1]).toMatchObject({
      fan_out_revision: 9,
      items: [{ node_id: "node-b", generation: 3 }],
    });
    expect(
      ciphertextBodies[1]?.items,
    ).toHaveLength(1);
    expect(screen.queryByText("decrypt_failed")).not.toBeInTheDocument();
    expect(screen.getAllByText("consumed")).toHaveLength(2);

    const serializedCalls = JSON.stringify(calls);
    expect(serializedCalls).not.toContain(secret);
    expect(input.value).toBe("");
    expect(root.textContent).not.toContain(secret);
    expect(Array.from(storage.values.values()).join("\n")).not.toContain(secret);
    expect(window.location.href).not.toContain(secret);
    expect(cryptoMock.capturedPlaintexts).toHaveLength(3);
    for (const plaintext of cryptoMock.capturedPlaintexts) {
      expect([...plaintext]).toEqual(Array(plaintext.length).fill(0));
    }
  });
});
