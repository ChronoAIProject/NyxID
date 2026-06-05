import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CredentialAcceptPage } from "./credential-accept";

const { hooks, mockNavigate, routerState } = vi.hoisted(() => ({
  hooks: {
    node: {
      data: {
        capabilities: { remote_credential_crypto_v1: true },
      },
      isLoading: false,
      error: null as unknown,
    },
  },
  mockNavigate: vi.fn(),
  routerState: {
    params: { nodeId: "node-1", pendingId: "pending-1" } as {
      nodeId?: string;
      pendingId: string;
    },
    search: {} as { return_to?: string },
  },
}));

const {
  capturedPlaintexts,
  mockBuildRciContext,
  mockEncrypt,
  mockGet,
  mockPost,
  mockToastInfo,
  mockToastSuccess,
} = vi.hoisted(() => ({
  capturedPlaintexts: [] as Uint8Array[],
  mockBuildRciContext: vi.fn((fields: Record<string, unknown>) => ({
    ...fields,
    version: fields.version ?? "v1",
    kdfInfoBytes: () => new Uint8Array(),
    aadBytes: () => new Uint8Array(),
  })),
  mockEncrypt: vi.fn(
    (
      plaintext: Uint8Array,
      _nodePubkey: string,
      context: { readonly node_id?: unknown },
    ) => {
      capturedPlaintexts.push(plaintext);
      return {
        version: "v1",
        admin_pubkey: `admin-${String(context.node_id)}`,
        nonce: `nonce-${String(context.node_id)}`,
        ciphertext: `cipher-${String(context.node_id)}`,
      };
    },
  ),
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockToastInfo: vi.fn(),
  mockToastSuccess: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => routerState.params,
  useSearch: () => routerState.search,
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNode: () => hooks.node,
}));

vi.mock("@/lib/api-client", () => {
  class ApiError extends Error {
    readonly status: number;
    readonly errorCode: number;

    constructor(status: number, response: { error_code: number; message: string }) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  }

  return {
    ApiError,
    api: { get: mockGet, post: mockPost },
  };
});

vi.mock("@/lib/crypto", () => ({
  MAX_CIPHERTEXT_SIZE: 16 * 1024,
  VERSION_V1: "v1",
  buildRciContext: mockBuildRciContext,
  encrypt: mockEncrypt,
}));

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: () => {},
}));

vi.mock("sonner", () => ({
  toast: { info: mockToastInfo, success: mockToastSuccess },
}));

beforeEach(() => {
  vi.clearAllMocks();
  capturedPlaintexts.length = 0;
  localStorage.clear();
  routerState.search = {};
  routerState.params = { nodeId: "node-1", pendingId: "pending-1" };
  hooks.node = {
    data: {
      capabilities: { remote_credential_crypto_v1: true },
    },
    isLoading: false,
    error: null,
  };
});

afterEach(() => {
  vi.restoreAllMocks();
});

async function clickBack() {
  const user = userEvent.setup();
  render(<CredentialAcceptPage />);

  await user.click(screen.getByRole("button", { name: "Back" }));
}

function mockFanOutGet() {
  mockGet.mockImplementation((path: string) => {
    if (path === "/nodes/credentials/pending/fanout-1/fan-out") {
      return Promise.resolve({
        fanout_id: "fanout-1",
        fan_out_revision: 7,
        service_slug: "openclaw",
        injection_method: "header",
        field_name: "X-API-Key",
        target_url: null,
        label: "Production",
        target_count: 2,
        remote_state: "pubkey_posted",
        created_at: "2026-06-05T00:00:00Z",
        expires_at: "2026-06-05T01:00:00Z",
        targets: [],
      });
    }
    if (path === "/nodes/credentials/pending/fanout-1/fan-out/pubkeys") {
      return Promise.resolve({
        fanout_id: "fanout-1",
        fan_out_revision: 7,
        target_count: 2,
        targets: [
          {
            node_id: "node-a",
            generation: 0,
            version: "v1",
            node_pubkey: "pubkey-a",
            remote_state: "pubkey_posted",
          },
          {
            node_id: "node-b",
            generation: 2,
            version: "v1",
            node_pubkey: "pubkey-b",
            remote_state: "pubkey_posted",
          },
        ],
      });
    }
    throw new Error(`unexpected GET ${path}`);
  });
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
        error_code: null,
        error_kind: null,
        delivery_status: "sent",
      },
      {
        node_id: "node-b",
        generation: 2,
        remote_state: "decrypt_failed",
        error_code: 8006,
        error_kind: "pending_credential_decrypt_failed",
        delivery_status: "sent",
      },
    ],
  };
}

async function submitSecret(secret: string) {
  const user = userEvent.setup();
  render(<CredentialAcceptPage />);
  await user.type(screen.getByLabelText("Credential value"), secret);
  await user.click(screen.getByRole("button", { name: "Accept" }));
  return user;
}

describe("CredentialAcceptPage return_to redirect guard", () => {
  it("honors a normal relative return_to path", async () => {
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    routerState.search = { return_to: "/nodes/abc" };

    await clickBack();

    expect(assignSpy).toHaveBeenCalledWith("/nodes/abc");
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it.each(["//evil.example", "/\\evil.example", "https://evil.example"])(
    "falls back instead of assigning unsafe return_to %s",
    async (returnTo) => {
      const assignSpy = vi
        .spyOn(window.location, "assign")
        .mockImplementation(() => undefined);
      routerState.search = { return_to: returnTo };

      await clickBack();

      expect(assignSpy).not.toHaveBeenCalled();
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/nodes/$nodeId",
        params: { nodeId: "node-1" },
      });
    },
  );
});

describe("CredentialAcceptPage fan-out accept", () => {
  it("encrypts once per pubkey and posts one aggregate fan-out request", async () => {
    routerState.params = { pendingId: "fanout-1" };
    mockFanOutGet();
    mockPost.mockResolvedValueOnce(fanOutPartialResponse());

    await submitSecret("super-secret-plaintext-fixture");

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledTimes(1);
    });
    expect(mockEncrypt).toHaveBeenCalledTimes(2);
    expect(mockBuildRciContext).toHaveBeenCalledWith({
      node_id: "node-a",
      pending_credential_id: "fanout-1",
      service_slug: "openclaw",
      injection_method: "header",
      field_name: "X-API-Key",
      target_url: null,
      version: "v1",
    });
    expect(mockBuildRciContext).toHaveBeenCalledWith({
      node_id: "node-b",
      pending_credential_id: "fanout-1",
      service_slug: "openclaw",
      injection_method: "header",
      field_name: "X-API-Key",
      target_url: null,
      version: "v1",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/nodes/credentials/pending/fanout-1/fan-out/ciphertexts",
      {
        fan_out_revision: 7,
        items: [
          {
            node_id: "node-a",
            generation: 0,
            version: "v1",
            admin_pubkey: "admin-node-a",
            nonce: "nonce-node-a",
            ciphertext: "cipher-node-a",
          },
          {
            node_id: "node-b",
            generation: 2,
            version: "v1",
            admin_pubkey: "admin-node-b",
            nonce: "nonce-node-b",
            ciphertext: "cipher-node-b",
          },
        ],
      },
    );
    expect(await screen.findByText("node-a")).toBeInTheDocument();
    expect(screen.getByText("node-b")).toBeInTheDocument();
    expect(screen.getByText("consumed")).toBeInTheDocument();
    expect(screen.getByText("decrypt_failed")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Retry failed" }),
    ).toBeInTheDocument();
  });

  it("posts retry-failed-targets with the latest fan-out revision", async () => {
    routerState.params = { pendingId: "fanout-1" };
    mockFanOutGet();
    mockPost
      .mockResolvedValueOnce(fanOutPartialResponse())
      .mockResolvedValueOnce({
        fanout_id: "fanout-1",
        fan_out_revision: 9,
        remote_state: null,
        targets: [
          {
            node_id: "node-b",
            generation: 3,
            remote_state: null,
            error_code: null,
            error_kind: null,
            delivery_status: null,
          },
        ],
      });

    const user = await submitSecret("super-secret-plaintext-fixture");
    await screen.findByRole("button", { name: "Retry failed" });
    await user.click(screen.getByRole("button", { name: "Retry failed" }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledTimes(2);
    });
    expect(mockPost).toHaveBeenLastCalledWith(
      "/nodes/credentials/pending/fanout-1/fan-out/retry-failed",
      { fan_out_revision: 8 },
    );
    expect(mockToastInfo).toHaveBeenCalledWith("1 failed target(s) reset");
    expect(screen.getByText("Ready")).toBeInTheDocument();
  });

  it("zeros plaintext and does not expose it in URL, storage, console, or DOM", async () => {
    routerState.params = { pendingId: "fanout-1" };
    mockFanOutGet();
    mockPost.mockResolvedValueOnce(fanOutPartialResponse());
    const consoleLog = vi.spyOn(console, "log").mockImplementation(() => {});
    const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    const secret = "super-secret-plaintext-fixture";

    await submitSecret(secret);
    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledTimes(1);
    });

    expect(capturedPlaintexts).toHaveLength(2);
    for (const plaintext of capturedPlaintexts) {
      expect([...plaintext]).toEqual(Array.from({ length: plaintext.length }, () => 0));
    }
    expect(screen.getByLabelText("Credential value")).toHaveValue("");
    expect(window.location.href).not.toContain(secret);
    expect(JSON.stringify(mockGet.mock.calls)).not.toContain(secret);
    expect(JSON.stringify(mockPost.mock.calls)).not.toContain(secret);
    const storedValues = Array.from({ length: localStorage.length }, (_, index) => {
      const key = localStorage.key(index);
      return key ? `${key}:${localStorage.getItem(key) ?? ""}` : "";
    }).join("\n");
    expect(storedValues).not.toContain(secret);
    const consoleText = [
      ...consoleLog.mock.calls,
      ...consoleWarn.mock.calls,
      ...consoleError.mock.calls,
    ]
      .flat()
      .join("\n");
    expect(consoleText).not.toContain(secret);
    expect(document.body.textContent ?? "").not.toContain(secret);
  });
});

describe("CredentialAcceptPage single-node accept", () => {
  it("keeps the single-node accept path working", async () => {
    mockGet.mockImplementation((path: string) => {
      if (path === "/nodes/node-1/credentials/pending/pending-1") {
        return Promise.resolve({
          pending_id: "pending-1",
          node_id: "node-1",
          service_slug: "openclaw",
          version: "v1",
          node_pubkey: "node-pubkey",
          remote_state: "pubkey_posted",
        });
      }
      if (path === "/nodes/node-1/credentials/pending?include_history=true") {
        return Promise.resolve({
          pending_credentials: [
            {
              id: "pending-1",
              node_id: "node-1",
              service_slug: "openclaw",
              injection_method: "header",
              field_name: "X-API-Key",
              target_url: null,
              label: "Production",
              created_at: "2026-06-05T00:00:00Z",
              expires_at: "2026-06-05T01:00:00Z",
              consumed_at: null,
              declined_at: null,
              is_active: true,
              remote_state: "pubkey_posted",
            },
          ],
        });
      }
      throw new Error(`unexpected GET ${path}`);
    });
    mockPost.mockResolvedValueOnce({
      delivery_status: "sent",
      remote_state: "consumed",
      error_code: null,
    });

    await submitSecret("single-node-secret");

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledTimes(1);
    });
    expect(mockEncrypt).toHaveBeenCalledTimes(1);
    expect(mockBuildRciContext).toHaveBeenCalledWith({
      node_id: "node-1",
      pending_credential_id: "pending-1",
      service_slug: "openclaw",
      injection_method: "header",
      field_name: "X-API-Key",
      target_url: null,
      version: "v1",
    });
    expect(mockPost).toHaveBeenCalledWith(
      "/nodes/node-1/credentials/pending/pending-1/ciphertext",
      {
        version: "v1",
        admin_pubkey: "admin-node-1",
        nonce: "nonce-node-1",
        ciphertext: "cipher-node-1",
      },
    );
    expect(mockToastSuccess).toHaveBeenCalledWith("Credential accepted");
    expect(screen.getByText("Stored")).toBeInTheDocument();
  });
});
