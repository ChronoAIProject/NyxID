import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiKeyCreateResponse } from "@/types/api";
import { ConnectVerifyStep, type CreatedKey } from "./connect-verify-step";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";

/**
 * Regression fence for two bugs:
 *
 * 1. The silent CTA (2026-06-30) — Footer's dispatch used to return
 *    the handler reference without invoking it. See the "REGRESSION"
 *    test below.
 *
 * 2. The false-negative probe (2026-07-01) — the probe used to define
 *    success as "downstream returned 2xx", which mis-classified
 *    services like telegram-bot (200 with bad creds → false positive)
 *    and Codex / GitHub PAT (401 with valid Agent Key → false negative).
 *    The fix keys off `X-NyxID-Agent-Id` on the response, which NyxID
 *    only stamps when the Agent Key + scope + route all resolved.
 */

const { createApiKeyMutate, toastFns } = vi.hoisted(() => ({
  createApiKeyMutate: vi.fn(),
  toastFns: { success: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/use-api-keys", () => ({
  useCreateApiKey: () => ({
    mutate: createApiKeyMutate,
    isPending: false,
  }),
}));

// CopyableField calls sonner.toast on copy — mock to no-op in test env.
vi.mock("sonner", () => ({ toast: toastFns }));

const CREATED_KEY: CreatedKey = {
  id: "svc-1",
  slug: "openai",
  catalogSlug: "openai",
  serviceName: "OpenAI",
  completionMode: "credential",
};

const MINTED_KEY: ApiKeyCreateResponse = {
  id: "ak-1",
  name: "Key for OpenAI",
  key_prefix: "nyxid_ag_abcd",
  full_key: "nyxid_ag_abcd_full_secret_value",
  scopes: ["proxy"],
  allow_all_services: false,
  allowed_service_ids: ["svc-1"],
  allow_all_nodes: true,
  allowed_node_ids: [],
  created_at: "2026-01-01T00:00:00Z",
  rate_limit_per_second: null,
  rate_limit_burst: null,
  platform: null,
} as unknown as ApiKeyCreateResponse;

/** Build a NyxID-proxied response — X-NyxID-Agent-Id present. */
function proxied(status: number): Response {
  return new Response("{}", {
    status,
    headers: { "x-nyxid-agent-id": "ak-1" },
  });
}

/** Build a NyxID-rejected response — no agent-id header. */
function rejected(status: number): Response {
  return new Response("{}", { status });
}

async function mintKey(user: ReturnType<typeof userEvent.setup>) {
  createApiKeyMutate.mockImplementation((_params, opts) => {
    opts?.onSuccess?.(MINTED_KEY);
  });
  await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled(),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(window, "fetch").mockReset();
});

describe("ConnectVerifyStep — initial phase (connected)", () => {
  it("renders the 'you need an Agent Key' prompt and both CTAs", () => {
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    expect(document.body.textContent ?? "").toMatch(
      /need an\s+Agent Key\s+to call/i,
    );
    expect(
      screen.getByRole("button", { name: /Create Agent Key/i }),
    ).toBeEnabled();
    expect(screen.getByRole("button", { name: /Maybe later/i })).toBeEnabled();
  });

  it("REGRESSION (silent CTA): clicking 'Create Agent Key' fires the mutation", async () => {
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    // If dispatch drifts back to "return function reference" form, this
    // fails immediately with 0 calls instead of 1.
    expect(createApiKeyMutate).toHaveBeenCalledTimes(1);
    expect(createApiKeyMutate).toHaveBeenCalledWith(
      {
        name: "Key for OpenAI",
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: ["svc-1"],
        allow_all_nodes: true,
      },
      expect.objectContaining({
        onSuccess: expect.any(Function),
        onError: expect.any(Function),
      }),
    );
  });

  it("'Maybe later' calls onDone without firing the mutation", async () => {
    const onDone = vi.fn();
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={onDone}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Maybe later/i }));

    expect(onDone).toHaveBeenCalledTimes(1);
    expect(createApiKeyMutate).not.toHaveBeenCalled();
  });
});

describe("ConnectVerifyStep — key_ready phase", () => {
  it("renders the AgentKeyPanel with Test / Wire buttons after mint succeeds", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    await waitFor(() =>
      expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Test Agent Key/i })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: /I'll wire it myself/i }),
    ).toBeEnabled();
  });

  it("mint failure surfaces the server error message + a Try again button", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onError?.(new Error("scoped-service-must-be-active"));
    });
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    await waitFor(() =>
      expect(
        screen.getByText(/scoped-service-must-be-active/i),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: /Try again/i })).toBeEnabled();
  });
});

describe("ConnectVerifyStep — probe (downstream OK)", () => {
  it("200 with X-NyxID-Agent-Id → end-to-end verified, dispatches success event", async () => {
    const fetchMock = vi.spyOn(window, "fetch").mockResolvedValue(proxied(200));

    const succeededSpy = vi.fn();
    window.addEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const firstCall = fetchMock.mock.calls[0];
    if (!firstCall) throw new Error("fetch was not called");
    const [url, init] = firstCall;
    expect(String(url)).toBe("/api/v1/proxy/s/openai/v1/models");
    expect(init).toMatchObject({
      method: "GET",
      credentials: "omit",
      headers: expect.objectContaining({
        Authorization: `Bearer ${MINTED_KEY.full_key}`,
      }),
    });

    await waitFor(() =>
      expect(screen.getByText(/End-to-end verified/i)).toBeInTheDocument(),
    );
    expect(succeededSpy).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();

    window.removeEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);
  });
});

describe("ConnectVerifyStep — probe (agent key VALID despite downstream failure)", () => {
  it("NEW SEMANTICS: 401 WITH agent-id header → agent key valid, warn tone, still fires success event", async () => {
    // This is the case the OLD probe got wrong: telegram-bot,
    // openclaw, and any service where a 4xx from downstream doesn't
    // mean the Agent Key is broken. The header proves NyxID accepted
    // the key + scope + reached downstream — the downstream's own
    // 401 is a stored-credential issue, not an agent-key issue.
    vi.spyOn(window, "fetch").mockResolvedValue(proxied(401));

    const succeededSpy = vi.fn();
    window.addEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    // "Agent Key works" copy MUST appear even though the downstream
    // returned 401 — this is the whole point of the fix.
    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(/Agent Key works/i),
    );
    expect(document.body.textContent ?? "").toMatch(
      /rejected the stored credential/i,
    );
    // Success event still fires — the aha-moment activation flag flips
    // as soon as the Agent Key is provably valid.
    expect(succeededSpy).toHaveBeenCalledTimes(1);
    // Done, not Retry — because the key IS valid.
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();

    window.removeEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);
  });

  it("NEW SEMANTICS: 404 WITH agent-id header → agent key valid, does-not-mean-key-is-broken copy", async () => {
    vi.spyOn(window, "fetch").mockResolvedValue(proxied(404));

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(
        /does not mean the key is broken/i,
      ),
    );
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
  });
});

describe("ConnectVerifyStep — probe (agent key INVALID: NyxID-layer rejection)", () => {
  it("401 WITHOUT agent-id header → NyxID rejected the Agent Key, transitions to probe_failed", async () => {
    vi.spyOn(window, "fetch").mockResolvedValue(rejected(401));

    const succeededSpy = vi.fn();
    window.addEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(/rejected the Agent Key/i),
    );
    // Success event must NOT fire when the key was rejected.
    expect(succeededSpy).not.toHaveBeenCalled();
    // Retry test button, not Done — the user needs to fix the key.
    expect(screen.getByRole("button", { name: /Retry test/i })).toBeEnabled();
    // Panel stays visible so the user can grab the (broken) key and
    // debug in devtools if needed.
    expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument();

    window.removeEventListener(FIRST_PROXY_CALL_SUCCEEDED_EVENT, succeededSpy);
  });

  it("403 WITHOUT agent-id header → scope violation, diagnostic names the slug", async () => {
    vi.spyOn(window, "fetch").mockResolvedValue(rejected(403));

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(/out-of-scope/i),
    );
    expect(document.body.textContent ?? "").toMatch(/`openai`/);
  });

  it("network error → timed-out diagnostic, agent key remains unverified", async () => {
    vi.spyOn(window, "fetch").mockRejectedValue(new TypeError("network down"));

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(
        /timed out|blocked|network/i,
      ),
    );
    expect(screen.getByRole("button", { name: /Retry test/i })).toBeEnabled();
  });
});

describe("ConnectVerifyStep — loading events + Done", () => {
  it("Test click dispatches loading start + end events regardless of probe outcome", async () => {
    vi.spyOn(window, "fetch").mockRejectedValue(new TypeError("network error"));

    const startSpy = vi.fn();
    const endSpy = vi.fn();
    window.addEventListener(VERIFY_KEY_LOADING_START_EVENT, startSpy);
    window.addEventListener(VERIFY_KEY_LOADING_END_EVENT, endSpy);

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() => expect(endSpy).toHaveBeenCalledTimes(1));
    expect(startSpy).toHaveBeenCalledTimes(1);

    window.removeEventListener(VERIFY_KEY_LOADING_START_EVENT, startSpy);
    window.removeEventListener(VERIFY_KEY_LOADING_END_EVENT, endSpy);
  });

  it("shell env snippet renders only on probe_success for openai-shaped slugs", async () => {
    vi.spyOn(window, "fetch").mockResolvedValue(proxied(200));

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await mintKey(user);

    // Not visible before the probe runs.
    expect(screen.queryByText("Shell env")).toBeNull();

    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));

    await waitFor(() =>
      expect(screen.getByText("Shell env")).toBeInTheDocument(),
    );
    const envCode = document.body.textContent ?? "";
    expect(envCode).toMatch(/OPENAI_API_KEY="nyxid_ag_abcd_full_secret_value"/);
    expect(envCode).toMatch(/OPENAI_BASE_URL=/);
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
  });

  it("Done in probe_success calls onDone", async () => {
    vi.spyOn(window, "fetch").mockResolvedValue(proxied(200));

    const onDone = vi.fn();
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CREATED_KEY}
        isNodeRouted={false}
        onDone={onDone}
      />,
    );
    await mintKey(user);
    await user.click(screen.getByRole("button", { name: /Test Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /^Done$/i }));

    expect(onDone).toHaveBeenCalledTimes(1);
  });
});
