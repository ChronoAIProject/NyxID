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
  slug: "llm-openai",
  catalogSlug: "llm-openai",
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

  it("mint failure surfaces the server error message + a Try again button + hides the panel", async () => {
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
    // GLM finding #9 — silent-pass trap. The AgentKeyPanel must NOT
    // render when mint failed (no minted secret to display). If a
    // regression accidentally mounts the panel with a null agent key,
    // the secret would appear as `undefined` or crash silently.
    expect(screen.queryByText("Your new Agent Key")).toBeNull();
    expect(
      screen.queryByRole("button", { name: /Test Agent Key/i }),
    ).toBeNull();
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
    expect(String(url)).toBe("/api/v1/proxy/s/llm-openai/v1/models");
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
    expect(document.body.textContent ?? "").toMatch(/`llm-openai`/);
  });

  it("REGRESSION: Retry test click re-invokes the probe (Footer dispatch works across phase transitions)", async () => {
    // Kimi finding — the initial dispatch was tested, but nothing
    // pinned that dispatch STAYS wired after phase transitions.
    // First probe: 401 without header → probe_failed with Retry test.
    // Click Retry: probe fires again. Second probe: 200 with header →
    // probe_success. Confirms dispatch survives re-render.
    const fetchMock = vi
      .spyOn(window, "fetch")
      .mockResolvedValueOnce(new Response("{}", { status: 401 })) // fails
      .mockResolvedValueOnce(
        new Response("{}", {
          status: 200,
          headers: { "x-nyxid-agent-id": "ak-1" },
        }),
      );

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
      expect(screen.getByRole("button", { name: /Retry test/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /Retry test/i }));

    // Two fetch calls, second one landed on success.
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(document.body.textContent ?? "").toMatch(/End-to-end verified/i),
    );
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
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

describe("ConnectVerifyStep — no Test button when we can't be highly confident it works", () => {
  // Registered untestable (Codex chat-only API, no cheap GET).
  const CODEX_KEY: CreatedKey = {
    ...CREATED_KEY,
    slug: "llm-openai-codex",
    catalogSlug: "llm-openai-codex",
    serviceName: "OpenAI Codex API",
    completionMode: "device_code",
  };

  // Unregistered custom endpoint — we don't know its API shape at all,
  // so we shouldn't offer a probe that might mislead the user.
  const CUSTOM_KEY: CreatedKey = {
    ...CREATED_KEY,
    slug: "acme-internal-api",
    catalogSlug: "acme-internal-api",
    serviceName: "Acme Internal API",
    completionMode: "credential",
  };

  it("registered-untestable (Codex): 'not supported' hint + only Done button, no probe fires", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const fetchSpy = vi.spyOn(window, "fetch");

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CODEX_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    // Panel appears — user can still copy the key.
    await waitFor(() =>
      expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument(),
    );
    // Hint text explains WHY there's no Test button, name-checks the service.
    expect(document.body.textContent ?? "").toMatch(
      /Automatic testing isn't supported/i,
    );
    expect(document.body.textContent ?? "").toMatch(/OpenAI Codex API/);

    // The Test Agent Key button MUST NOT render — a fake green from a
    // probe we can't trust is worse than no probe at all.
    expect(
      screen.queryByRole("button", { name: /Test Agent Key/i }),
    ).toBeNull();
    // Single primary CTA — Done. No "I'll wire it myself" either.
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: /I'll wire it myself/i }),
    ).toBeNull();

    // Sanity: no probe fired.
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("unregistered custom endpoint: also hides the Test button (high-confidence rule)", async () => {
    // The tighter rule (Calvin, 2026-07-01): if we don't have an
    // explicit recipe for this slug in PROBE_REGISTRY, we don't offer
    // the Test button — even the header-based safety-net probe can
    // return misleading yellow-warn states we haven't validated
    // against real user endpoints. Better to say nothing.
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const fetchSpy = vi.spyOn(window, "fetch");

    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CUSTOM_KEY}
        isNodeRouted={false}
        onDone={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));

    await waitFor(() =>
      expect(screen.getByText(MINTED_KEY.full_key)).toBeInTheDocument(),
    );
    expect(document.body.textContent ?? "").toMatch(
      /Automatic testing isn't supported/i,
    );
    expect(document.body.textContent ?? "").toMatch(/Acme Internal API/);
    expect(
      screen.queryByRole("button", { name: /Test Agent Key/i }),
    ).toBeNull();
    // Kimi parity check — both `Test Agent Key` AND `I'll wire it
    // myself` must be hidden for untestable/unregistered slugs. The
    // footer collapses to a single Done CTA in either case.
    expect(
      screen.queryByRole("button", { name: /I'll wire it myself/i }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("Done from key_ready calls onDone on untestable providers", async () => {
    createApiKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.(MINTED_KEY);
    });
    const onDone = vi.fn();
    const user = userEvent.setup();
    render(
      <ConnectVerifyStep
        createdKey={CODEX_KEY}
        isNodeRouted={false}
        onDone={onDone}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Create Agent Key/i }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Done$/i })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: /^Done$/i }));

    expect(onDone).toHaveBeenCalledTimes(1);
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
