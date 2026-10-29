import {
  onlineManager,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ServiceMonitoring } from "./service-monitoring";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type {
  MonitoringResponse,
  MonitoringResult,
} from "@/schemas/monitoring";
let response: MonitoringResponse;
let calls: { path: string; method: string; body: unknown }[];
let failure: { status: number; message: string } | undefined;
const observation = (): MonitoringResult => ({
  service: { id: "exact-id" },
  health: {
    status: "healthy",
    source: "owner",
    path: "healthz",
    reason: "ok",
    checked_at: new Date().toISOString(),
    stale_at: new Date(Date.now() + 60_000).toISOString(),
    freshness: "fresh",
    duration_ms: 4,
    status_code: 200,
  },
  readiness: null,
});
beforeEach(() => {
  calls = [];
  failure = undefined;
  useAuthStore.setState({ user: { id: "owner" } as User });
  response = {
    service_id: "exact-id",
    can_manage: true,
    can_probe: true,
    status: observation(),
    configuration: {
      user_service_id: "exact-id",
      revision: 7,
      owner_config: {
        health: { path: "healthz", expect_status: [200] },
        readiness: null,
      },
    },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: string, options?: RequestInit) => {
      const method = options?.method ?? "GET";
      const path = String(input);
      calls.push({
        path,
        method,
        body: options?.body ? JSON.parse(String(options.body)) : undefined,
      });
      const failed = failure;
      const data = failed
        ? {
            error: "monitoring_error",
            error_code: 13300,
            message: failed.message,
          }
        : method === "GET"
          ? response
          : path.endsWith("/probe") || path.endsWith("/validate")
            ? observation()
            : { ...response.configuration, revision: 8 };
      return new Response(JSON.stringify(data), {
        status: failed?.status ?? 200,
        headers: { "Content-Type": "application/json" },
      });
    }),
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ServiceMonitoring serviceId="exact-id" />
    </QueryClientProvider>,
  );
}
it("page load performs one scoped read and Check now is explicit", async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByText("Healthy");
  expect(calls).toEqual([
    {
      path: "/api/v1/keys/exact-id/monitoring",
      method: "GET",
      body: undefined,
    },
  ]);
  await user.click(screen.getByRole("button", { name: "Check now" }));
  await screen.findByText("Manual check result");
  expect(
    calls.filter((call) => call.method === "POST").map((call) => call.path),
  ).toEqual(["/api/v1/keys/exact-id/monitoring/probe"]);
});
it("members can view and check without manager controls", async () => {
  response.can_manage = false;
  response.configuration = null;
  mount();
  await screen.findByText("Healthy");
  expect(screen.getByRole("button", { name: "Check now" })).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Configure" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Clear configuration" }),
  ).not.toBeInTheDocument();
});
it("tests a draft without saving, then saves with exact revision", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole("button", { name: "Configure" }));
  const dialog = within(screen.getByRole("dialog"));
  expect(dialog.getByRole("button", { name: "Save" })).toBeDisabled();
  await user.clear(dialog.getByLabelText("Relative GET path"));
  await user.type(dialog.getByLabelText("Relative GET path"), "health/ready");
  await user.click(dialog.getByRole("button", { name: "Test draft" }));
  await dialog.findByText("Draft test result (not saved)");
  expect(calls.filter((call) => call.method !== "GET")).toEqual([
    {
      path: "/api/v1/keys/exact-id/monitoring/validate",
      method: "POST",
      body: {
        health: { path: "health/ready", expect_status: [200] },
        readiness: null,
      },
    },
  ]);
  await user.click(dialog.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(calls.find((call) => call.method === "PUT")?.body).toEqual({
    revision: 7,
    health: { path: "health/ready", expect_status: [200] },
    readiness: null,
  });
});
it("clears only after confirmation and preserves revision", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(
    await screen.findByRole("button", { name: "Clear configuration" }),
  );
  const dialog = within(screen.getByRole("dialog"));
  expect(
    dialog.getByText(/built-in or discovered health check may take over/),
  ).toBeVisible();
  expect(calls.some((call) => call.method === "DELETE")).toBe(false);
  await user.click(dialog.getByRole("button", { name: "Clear configuration" }));
  await waitFor(() =>
    expect(calls.some((call) => call.method === "DELETE")).toBe(true),
  );
  expect(calls.find((call) => call.method === "DELETE")?.path).toBe(
    "/api/v1/keys/exact-id/monitoring?revision=7",
  );
});
it("surfaces conflicts without retry or overwrite", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(
    await screen.findByRole("button", { name: "Clear configuration" }),
  );
  failure = { status: 409, message: "Configuration changed; reload" };
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "Clear configuration",
    }),
  );
  await screen.findByRole("button", { name: "Reload configuration" });
  expect(screen.getByRole("alert")).toHaveTextContent("Configuration changed");
  expect(calls.filter((call) => call.method === "DELETE")).toHaveLength(1);
});
it("an unavailable refresh hides old healthy status", async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByText("Healthy");
  failure = { status: 503, message: "Monitoring unavailable" };
  await user.click(
    screen.getByRole("button", { name: "Refresh monitoring status" }),
  );
  await screen.findByText(/Health and readiness are unknown/);
  expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
});
it("stale observations display Unknown with original reason", async () => {
  response.status.health.freshness = "stale";
  mount();
  await screen.findByText("Stale or unknown");
  expect(
    within(screen.getByRole("region", { name: "Health" })).getAllByText(
      "Unknown",
    )[0],
  ).toBeVisible();
  expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
  expect(screen.getByText("ok")).toBeVisible();
});
it("account changes cannot publish late owner authority", async () => {
  let resolve!: (response: Response) => void;
  vi.mocked(fetch).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  mount();
  await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
  act(() => useAuthStore.setState({ user: { id: "next-owner" } as User }));
  response.can_manage = false;
  response.configuration = null;
  await act(async () => {
    resolve(
      new Response(JSON.stringify({ ...response, can_manage: true }), {
        status: 200,
      }),
    );
  });
  await screen.findByText("Healthy");
  expect(
    screen.queryByRole("button", { name: "Configure" }),
  ).not.toBeInTheDocument();
});

it("invalidates a successful draft test when fields or toggles change", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole("button", { name: "Configure" }));
  const dialog = within(screen.getByRole("dialog"));
  await user.click(dialog.getByRole("button", { name: "Test draft" }));
  await dialog.findByText("Draft test result (not saved)");
  await user.type(dialog.getByLabelText("Relative GET path"), "/different");
  expect(
    dialog.queryByText("Draft test result (not saved)"),
  ).not.toBeInTheDocument();
  await user.click(dialog.getByRole("button", { name: "Test draft" }));
  await dialog.findByText("Draft test result (not saved)");
  await user.click(
    dialog.getAllByRole("switch", { name: "Use owner configuration" })[0]!,
  );
  expect(
    dialog.queryByText("Draft test result (not saved)"),
  ).not.toBeInTheDocument();
  expect(dialog.getByRole("button", { name: "Test draft" })).toBeDisabled();
  await user.click(dialog.getByRole("button", { name: "Cancel" }));
  await user.click(screen.getByRole("button", { name: "Configure" }));
  expect(
    within(screen.getByRole("dialog")).queryByText(
      "Draft test result (not saved)",
    ),
  ).not.toBeInTheDocument();
});

it("retains the edited draft after save conflict until the user reloads", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole("button", { name: "Configure" }));
  const dialog = within(screen.getByRole("dialog"));
  await user.type(dialog.getByLabelText("Relative GET path"), "/edited");
  failure = { status: 409, message: "Configuration changed; reload" };
  await user.click(dialog.getByRole("button", { name: "Save" }));
  await dialog.findByRole("button", { name: "Reload configuration" });
  expect(dialog.getByLabelText("Relative GET path")).toHaveValue(
    "healthz/edited",
  );
  expect(calls.filter((call) => call.method === "PUT")).toHaveLength(1);
  failure = undefined;
  response.configuration!.revision = 11;
  await user.click(
    dialog.getByRole("button", { name: "Reload configuration" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(calls.filter((call) => call.method === "GET")).toHaveLength(2);
});

it("renders independent readiness when health was not tested", async () => {
  const user = userEvent.setup();
  response.configuration!.owner_config = {
    health: null,
    readiness: { path: "readyz", expect_status: [204] },
  };
  mount();
  await user.click(await screen.findByRole("button", { name: "Configure" }));
  const dialog = within(screen.getByRole("dialog"));
  const result = observation();
  result.readiness = {
    ...result.health,
    status: "not_ready",
    path: "readyz",
    status_code: 503,
  };
  result.health = {
    ...result.health,
    status: "unknown",
    source: "none",
    path: "",
    checked_at: null,
    stale_at: null,
    freshness: "unknown",
  };
  vi.mocked(fetch).mockResolvedValueOnce(
    new Response(JSON.stringify(result), { status: 200 }),
  );
  await user.click(dialog.getByRole("button", { name: "Test draft" }));
  await dialog.findByText("Not ready");
  expect(
    within(dialog.getByRole("region", { name: "Draft health" })).getAllByText(
      "Unknown",
    )[0],
  ).toBeVisible();
  expect(dialog.queryByText("Healthy")).not.toBeInTheDocument();
});

it("ages a displayed badge without any network polling", async () => {
  mount();
  await screen.findByText("Healthy");
  vi.useFakeTimers();
  try {
    // The component's timer was installed before fake timers; remount it with
    // the same real response, then control the 10-second display cadence.
    cleanup();
    mount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(screen.getByText("Healthy")).toBeVisible();
    const requests = vi.mocked(fetch).mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(70_000);
    });
    expect(screen.queryByText("Healthy")).not.toBeInTheDocument();
    expect(vi.mocked(fetch)).toHaveBeenCalledTimes(requests);
  } finally {
    vi.useRealTimers();
  }
});

it("preserves a dirty draft and captured revision across network reconnect", async () => {
  const user = userEvent.setup();
  mount();
  await user.click(await screen.findByRole("button", { name: "Configure" }));
  const dialog = within(screen.getByRole("dialog"));
  await user.clear(dialog.getByLabelText("Relative GET path"));
  await user.type(
    dialog.getByLabelText("Relative GET path"),
    "health/local-draft",
  );
  response = {
    ...response,
    configuration: {
      ...response.configuration!,
      revision: 99,
      owner_config: {
        health: { path: "other-manager", expect_status: [200] },
        readiness: null,
      },
    },
  };
  await act(async () => {
    onlineManager.setOnline(false);
    onlineManager.setOnline(true);
  });
  expect(dialog.getByLabelText("Relative GET path")).toHaveValue(
    "health/local-draft",
  );
  expect(calls.filter((call) => call.method === "GET")).toHaveLength(1);
  await user.click(dialog.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(calls.some((call) => call.method === "PUT")).toBe(true),
  );
  expect(calls.find((call) => call.method === "PUT")?.body).toMatchObject({
    revision: 7,
    health: { path: "health/local-draft" },
  });
});

it("shows missing readiness evidence as Unknown even with an owner check", async () => {
  response.configuration!.owner_config.readiness = {
    path: "readyz",
    expect_status: [200],
  };
  response.status.readiness = null;
  mount();
  await screen.findByText("Healthy");
  const readiness = within(screen.getByRole("region", { name: "Readiness" }));
  expect(readiness.getAllByText("Unknown")[0]).toBeVisible();
  expect(readiness.queryByText("Not configured")).not.toBeInTheDocument();
});

it("reserves Not configured for explicit upstream readiness metadata", async () => {
  response.status.readiness = {
    ...observation().health,
    status: "not_configured",
    source: "none",
    path: "",
    checked_at: null,
    stale_at: null,
    freshness: "unknown",
  };
  mount();
  const readiness = within(
    await screen.findByRole("region", { name: "Readiness" }),
  );
  expect(readiness.getByText("Not configured")).toBeVisible();
});
