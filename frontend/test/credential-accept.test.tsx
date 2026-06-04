import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { generateEphemeralKeypair } from "@/lib/crypto";
import { encodeBase64UrlNoPad } from "@/lib/crypto/base64url";
import { CredentialAcceptPage } from "@/pages/credential-accept";

const NODE_ID = "node-1";
const PENDING_ID = "pending-1";
const SECRET = "sk-live-browser-secret";
const PUBKEY_ENDPOINT = `/nodes/${NODE_ID}/credentials/pending/${PENDING_ID}`;
const LIST_ENDPOINT = `/nodes/${NODE_ID}/credentials/pending?include_history=true`;

const { api, ApiError, navigateMock, routerState, nodeState, toastSuccess } =
  vi.hoisted(() => {
    class ApiError extends Error {
      status: number;
      errorCode: number;
      errorResponse: { error: string; error_code: number; message: string };

      constructor(
        status: number,
        response: { error: string; error_code: number; message: string },
      ) {
        super(response.message);
        this.name = "ApiError";
        this.status = status;
        this.errorCode = response.error_code;
        this.errorResponse = response;
      }
    }

    return {
      api: { get: vi.fn(), post: vi.fn() },
      ApiError,
      navigateMock: vi.fn(),
      routerState: {
        params: { nodeId: "node-1", pendingId: "pending-1" },
        search: {} as { return_to?: string },
      },
      nodeState: {
        result: {
          data: {
            id: "node-1",
            capabilities: { remote_credential_crypto_v1: true },
          },
          isLoading: false,
          error: null,
        },
      },
      toastSuccess: vi.fn(),
    };
  });

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigateMock,
  useParams: () => routerState.params,
  useSearch: () => routerState.search,
}));

vi.mock("@/hooks/use-nodes", () => ({
  useNode: () => nodeState.result,
}));

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({ api, ApiError }));

vi.mock("sonner", () => ({
  toast: { success: toastSuccess },
}));

function apiError(status: number, errorCode: number): Error {
  return new ApiError(status, {
    error: "pending_credential_pubkey_awaiting",
    error_code: errorCode,
    message: "Node public key is not available yet.",
  });
}

function pubkeyResponse() {
  const keypair = generateEphemeralKeypair();
  return {
    pending_id: PENDING_ID,
    node_id: NODE_ID,
    service_slug: "openai",
    version: "v1" as const,
    node_pubkey: encodeBase64UrlNoPad(keypair.publicKey),
  };
}

function pendingCredential(overrides: Record<string, unknown> = {}) {
  return {
    id: PENDING_ID,
    node_id: NODE_ID,
    service_slug: "openai",
    injection_method: "header",
    field_name: "Authorization",
    target_url: "https://api.openai.example",
    label: "OpenAI",
    created_by_user_id: "user-1",
    owner_user_id: "user-1",
    created_at: "2026-01-01T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
    is_active: true,
    ...overrides,
  };
}

function renderPage() {
  return render(<CredentialAcceptPage />);
}

async function flushAsyncWork() {
  for (let index = 0; index < 8; index += 1) {
    await Promise.resolve();
  }
}

function submitSecret(): HTMLInputElement {
  const input = screen.getByLabelText("Credential value") as HTMLInputElement;
  fireEvent.change(input, { target: { value: SECRET } });
  fireEvent.submit(input.closest("form")!);
  return input;
}

function localStorageDump(): string {
  const values: string[] = [];
  for (let index = 0; index < localStorage.length; index += 1) {
    const key = localStorage.key(index);
    if (key) {
      values.push(key, localStorage.getItem(key) ?? "");
    }
  }
  return values.join("\n");
}

function expectSecretNotLeaked(input: HTMLInputElement) {
  expect(input.value).toBe("");
  expect(window.location.href).not.toContain(SECRET);
  expect(localStorageDump()).not.toContain(SECRET);
  expect(document.body.textContent ?? "").not.toContain(SECRET);

  for (const [endpoint] of api.get.mock.calls) {
    expect(String(endpoint)).not.toContain(SECRET);
  }
  for (const [endpoint, body] of api.post.mock.calls) {
    expect(String(endpoint)).not.toContain(SECRET);
    expect(JSON.stringify(body)).not.toContain(SECRET);
  }

  for (const method of ["log", "warn", "error"] as const) {
    const calls = vi.mocked(console[method]).mock.calls;
    expect(JSON.stringify(calls)).not.toContain(SECRET);
  }
}

function configureSuccessFlow(finalPending = pendingCredential({ consumed_at: "2026-01-01T00:00:01Z" })) {
  let listCalls = 0;
  api.get.mockImplementation(async (endpoint: string) => {
    if (endpoint === PUBKEY_ENDPOINT) return pubkeyResponse();
    if (endpoint === LIST_ENDPOINT) {
      listCalls += 1;
      return {
        pending_credentials: [
          listCalls === 1 ? pendingCredential() : finalPending,
        ],
      };
    }
    throw new Error(`Unexpected GET ${endpoint}`);
  });
  api.post.mockResolvedValue({
    delivery_status: "sent",
    remote_state: "ciphertext_received",
  });
}

beforeEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  localStorage.clear();
  routerState.search = {};
  nodeState.result = {
    data: {
      id: NODE_ID,
      capabilities: { remote_credential_crypto_v1: true },
    },
    isLoading: false,
    error: null,
  };
  vi.spyOn(console, "log").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("CredentialAcceptPage", () => {
  it("encrypts and posts an envelope without leaking the secret", async () => {
    configureSuccessFlow();
    renderPage();

    const input = submitSecret();

    await waitFor(() => expect(screen.getByText("Stored")).toBeInTheDocument());
    expect(api.post).toHaveBeenCalledTimes(1);
    const [endpoint, envelope] = api.post.mock.calls[0];
    expect(endpoint).toBe(
      `/nodes/${NODE_ID}/credentials/pending/${PENDING_ID}/ciphertext`,
    );
    expect(envelope).toMatchObject({ version: "v1" });
    expect(String(envelope.admin_pubkey)).toMatch(/^[A-Za-z0-9_-]+$/u);
    expect(String(envelope.nonce)).toMatch(/^[A-Za-z0-9_-]+$/u);
    expect(String(envelope.ciphertext)).toMatch(/^[A-Za-z0-9_-]+$/u);
    expect(String(envelope.admin_pubkey)).not.toContain("=");
    expect(String(envelope.nonce)).not.toContain("=");
    expect(String(envelope.ciphertext)).not.toContain("=");
    expect(toastSuccess).toHaveBeenCalledWith("Credential accepted");
    expectSecretNotLeaked(input);
  });

  it("polls queued ciphertext delivery until the credential is stored", async () => {
    vi.useFakeTimers();
    let listCalls = 0;
    api.get.mockImplementation(async (endpoint: string) => {
      if (endpoint === PUBKEY_ENDPOINT) return pubkeyResponse();
      if (endpoint === LIST_ENDPOINT) {
        listCalls += 1;
        return {
          pending_credentials: [
            listCalls < 3
              ? pendingCredential({ remote_state: "ciphertext_queued" })
              : pendingCredential({
                  consumed_at: "2026-01-01T00:00:01Z",
                  remote_state: "consumed",
                }),
          ],
        };
      }
      throw new Error(`Unexpected GET ${endpoint}`);
    });
    api.post.mockResolvedValue({
      delivery_status: "queued",
      remote_state: "ciphertext_queued",
    });

    renderPage();
    let input: HTMLInputElement;
    await act(async () => {
      input = submitSecret();
      await flushAsyncWork();
    });

    expect(screen.getByText("Waiting for node")).toBeInTheDocument();
    expect(screen.getByText("queued")).toBeInTheDocument();
    expect(
      api.get.mock.calls.filter(([endpoint]) => endpoint === LIST_ENDPOINT),
    ).toHaveLength(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
      await flushAsyncWork();
    });

    expect(screen.getByText("Stored")).toBeInTheDocument();
    expect(toastSuccess).toHaveBeenCalledWith("Credential accepted");
    expect(
      api.get.mock.calls.filter(([endpoint]) => endpoint === LIST_ENDPOINT),
    ).toHaveLength(3);
    expect(
      api.get.mock.calls
        .filter(([endpoint]) =>
          String(endpoint).startsWith(`/nodes/${NODE_ID}`),
        )
        .every(
          ([endpoint]) =>
            endpoint === PUBKEY_ENDPOINT || endpoint === LIST_ENDPOINT,
        ),
    ).toBe(true);
    expect(api.post).toHaveBeenCalledWith(
      `/nodes/${NODE_ID}/credentials/pending/${PENDING_ID}/ciphertext`,
      expect.objectContaining({ version: "v1" }),
    );
    expectSecretNotLeaked(input!);
  });

  it("times out after polling non-terminal pending credential state without leaking the secret", async () => {
    vi.useFakeTimers();
    window.history.replaceState(
      null,
      "",
      `/nodes/${NODE_ID}/credentials/pending/${PENDING_ID}/accept?return_to=/nodes/${NODE_ID}`,
    );
    routerState.search = { return_to: `/nodes/${NODE_ID}` };
    let listCalls = 0;
    api.get.mockImplementation(async (endpoint: string) => {
      if (endpoint === PUBKEY_ENDPOINT) return pubkeyResponse();
      if (endpoint === LIST_ENDPOINT) {
        listCalls += 1;
        return {
          pending_credentials: [
            pendingCredential({ remote_state: "ciphertext_queued" }),
          ],
        };
      }
      throw new Error(`Unexpected GET ${endpoint}`);
    });
    api.post.mockResolvedValue({
      delivery_status: "queued",
      remote_state: "ciphertext_queued",
    });

    renderPage();
    let input: HTMLInputElement;
    await act(async () => {
      input = submitSecret();
      await flushAsyncWork();
    });

    expect(screen.getByText("Waiting for node")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
      await flushAsyncWork();
    });

    expect(screen.getByText("Timed out")).toBeInTheDocument();
    expect(
      screen.getByText(
        "The node did not report completion before the browser stopped waiting.",
      ),
    ).toBeInTheDocument();
    expect(listCalls).toBe(15);
    expect(
      api.get.mock.calls.filter(([endpoint]) => endpoint === LIST_ENDPOINT),
    ).toHaveLength(15);
    expect(window.location.href).not.toContain(SECRET);
    expect(window.location.search).not.toContain(SECRET);
    expect(JSON.stringify(routerState.search)).not.toContain(SECRET);
    expect(localStorageDump()).not.toContain(SECRET);
    expect(document.body.textContent ?? "").not.toContain(SECRET);
    expect(
      JSON.stringify([
        ...api.get.mock.calls.map(([endpoint]) => ({ endpoint })),
        ...api.post.mock.calls.map(([endpoint, body]) => ({ endpoint, body })),
      ]),
    ).not.toContain(SECRET);
    expect(
      JSON.stringify([
        ...vi.mocked(console.log).mock.calls,
        ...vi.mocked(console.warn).mock.calls,
        ...vi.mocked(console.error).mock.calls,
      ]),
    ).not.toContain(SECRET);
    expectSecretNotLeaked(input!);
  });

  it("backs off on 404 and 8009 before posting ciphertext", async () => {
    vi.useFakeTimers();
    let pubkeyCalls = 0;
    api.get.mockImplementation(async (endpoint: string) => {
      if (endpoint === PUBKEY_ENDPOINT) {
        pubkeyCalls += 1;
        if (pubkeyCalls === 1) throw apiError(404, 8000);
        if (pubkeyCalls === 2) throw apiError(400, 8009);
        return pubkeyResponse();
      }
      if (endpoint === LIST_ENDPOINT) {
        return {
          pending_credentials: [
            pendingCredential({ consumed_at: "2026-01-01T00:00:01Z" }),
          ],
        };
      }
      throw new Error(`Unexpected GET ${endpoint}`);
    });
    api.post.mockResolvedValue({
      delivery_status: "sent",
      remote_state: "ciphertext_received",
    });

    renderPage();
    let input: HTMLInputElement;
    await act(async () => {
      input = submitSecret();
      await Promise.resolve();
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
      await Promise.resolve();
    });
    expect(pubkeyCalls).toBe(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(api.post).toHaveBeenCalledTimes(1);
    expect(pubkeyCalls).toBe(3);
    expect(screen.getByText("Stored")).toBeInTheDocument();
    expectSecretNotLeaked(input!);
  });

  it("falls back to manual setup when the node lacks browser crypto capability", () => {
    nodeState.result = {
      data: {
        id: NODE_ID,
        capabilities: { remote_credential_crypto_v1: false },
      },
      isLoading: false,
      error: null,
    };
    renderPage();

    const input = submitSecret();

    expect(screen.getByText("Manual setup")).toBeInTheDocument();
    expect(api.get).not.toHaveBeenCalled();
    expect(api.post).not.toHaveBeenCalled();
    expectSecretNotLeaked(input);
  });

  it("falls back to manual setup when pubkey backoff times out", async () => {
    vi.useFakeTimers();
    api.get.mockRejectedValue(apiError(404, 8009));
    renderPage();

    const input = submitSecret();

    await act(async () => {
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(30_000);
      await Promise.resolve();
    });

    expect(screen.getByText("Manual setup")).toBeInTheDocument();
    expect(api.post).not.toHaveBeenCalled();
    expectSecretNotLeaked(input);
  });

  it.each([
    ["decrypt_failed", pendingCredential({ remote_state: "decrypt_failed" }), "Decrypt failed"],
    ["expired", pendingCredential({ expires_at: "2000-01-01T00:00:00Z" }), "Expired"],
    ["declined", pendingCredential({ declined_at: "2026-01-01T00:00:01Z" }), "Declined"],
  ])("maps polling terminal state %s", async (_name, terminalPending, label) => {
    configureSuccessFlow(terminalPending);
    renderPage();

    submitSecret();

    await waitFor(() => expect(screen.getByText(label)).toBeInTheDocument());
    expect(api.post).toHaveBeenCalledTimes(1);
  });

  it("returns to a safe same-origin return_to from the Back button", () => {
    routerState.search = { return_to: `/nodes/${NODE_ID}` };
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Back" }));

    expect(assignSpy).toHaveBeenCalledWith(`/nodes/${NODE_ID}`);
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("rejects a protocol-relative return_to (open-redirect guard) and navigates in-app", () => {
    // `//evil.example` passes a naive startsWith("/") check but is off-origin.
    routerState.search = { return_to: "//evil.example/phish" };
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    renderPage();

    fireEvent.click(screen.getByRole("button", { name: "Back" }));

    expect(assignSpy).not.toHaveBeenCalled();
    expect(navigateMock).toHaveBeenCalledWith({
      to: "/nodes/$nodeId",
      params: { nodeId: NODE_ID },
    });
  });
});
