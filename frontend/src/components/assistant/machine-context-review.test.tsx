import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { ApiError } from "@/lib/api-client";
import {
  machineAccessFixture,
  machineContextReviewFixture as snapshot,
  offCapabilities,
} from "@/test/machine-access-fixture";
import { MachineContextReview } from "./machine-context-review";

const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: request }));
const machine = {
  ...machineAccessFixture,
  machine_configuration: {
    ...snapshot.machine_configuration!,
    state: "applied",
    reason: null,
  },
};
let client: QueryClient;
let review = snapshot;
let conflict = false;
beforeEach(() => {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  useAuthStore.setState({ user: { id: "owner" } as User });
  review = { ...snapshot };
  conflict = false;
  request.mockReset().mockImplementation(async (path: string) => {
    if (path.endsWith("/commit")) {
      if (conflict)
        throw new ApiError(409, {
          error: "conflict",
          error_code: 1005,
          message: "machine_context_review_stale",
        });
      review = { ...review, status: "committed", committed_revision: 8 };
    }
    return review;
  });
});
afterEach(() => {
  cleanup();
  client.clear();
});
function show(
  props: Partial<React.ComponentProps<typeof MachineContextReview>> = {},
) {
  return render(
    <QueryClientProvider client={client}>
      <MachineContextReview
        machine={machine}
        agentId="agent"
        contextEnabled
        disabled={false}
        {...props}
      />
    </QueryClientProvider>,
  );
}
async function prepare() {
  fireEvent.click(
    screen.getByRole("button", {
      name: "Separate workspace and browser for this agent",
    }),
  );
  await screen.findByRole("region", { name: "Review environment mode change" });
  await waitFor(() =>
    expect(
      request.mock.calls.some(([path]) =>
        path.endsWith("/context-reviews/review-1"),
      ),
    ).toBe(true),
  );
}

it.each([offCapabilities, { ...offCapabilities, files: true }])(
  "reviews the stored exact scope without copying editable browser state (%j)",
  async (capabilities) => {
    review = { ...snapshot, capabilities, saved_login_ids: ["stored-login"] };
    show({
      machine: {
        ...machine,
        capabilities: { ...offCapabilities, shell: true },
        saved_login_ids: ["browser-state-login"],
      },
    });
    await prepare();
    const panel = screen.getByRole("region", {
      name: "Review environment mode change",
    });
    expect(
      within(panel).getByText(
        capabilities.files
          ? "Reviewed tools: File tools"
          : "Reviewed tools: None — all tools remain off",
      ),
    ).toBeVisible();
    expect(within(panel).getByText("stored-login")).toBeVisible();
    expect(
      within(panel).queryByText("browser-state-login"),
    ).not.toBeInTheDocument();
    expect(
      within(panel).getByText(/Tools added: None.*removed: None/),
    ).toBeVisible();
    const confirm = within(panel).getByRole("button", {
      name: "Approve and apply reviewed mode",
    });
    expect(confirm).toBeDisabled();
    fireEvent.click(
      within(panel).getByRole("checkbox", { name: /I approve this exact/ }),
    );
    await waitFor(() => expect(confirm).toBeEnabled());
    fireEvent.click(confirm);
    await screen.findByText(/Authorization committed at revision 8/);
    expect(
      screen.queryByText("Environment mode applied"),
    ).not.toBeInTheDocument();
    expect(
      request.mock.calls.filter(([path]) => path.endsWith("/commit"))[0]?.[1]
        ?.body,
    ).toEqual({});
  },
);

it("requires a fresh stored review and a new checkbox after stale commit conflict", async () => {
  show();
  await prepare();
  fireEvent.click(
    screen.getByRole("checkbox", { name: /I approve this exact/ }),
  );
  conflict = true;
  fireEvent.click(
    screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
  );
  await screen.findByText(/Review is stale.*prepare and review again/i);
  expect(
    screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
  ).toBeDisabled();
  review = {
    ...snapshot,
    review_id: "review-2",
    expected_revision: 8,
    saved_login_ids: [],
  };
  conflict = false;
  fireEvent.click(
    screen.getByRole("button", { name: "Prepare and review again" }),
  );
  await screen.findByText("Stored review review-2");
  expect(
    screen.getByRole("checkbox", { name: /I approve this exact/ }),
  ).not.toBeChecked();
  expect(
    request.mock.calls.filter(([path]) => path.endsWith("/commit")),
  ).toHaveLength(1);
});

it.each([
  { applicable: false, applicability_reason: "machine_context_review_stale" },
  { expires_at: "2020-01-01T00:00:00Z" },
  { status: "failed" as const, error_code: "machine_context_review_stale" },
])("refuses stale, expired or failed stored reviews %j", async (patch) => {
  review = { ...snapshot, ...patch };
  show();
  await prepare();
  expect(
    screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "Prepare and review again" }),
  ).toBeVisible();
  expect(
    request.mock.calls.filter(([path]) => path.endsWith("/commit")),
  ).toHaveLength(0);
});

it("requires explicit shared warning acknowledgement even when context opt-in is off", async () => {
  review = {
    ...snapshot,
    mode: "shared_legacy",
    previous_mode: "selection_required",
    saved_login_ids: [],
  };
  show({
    machine: {
      ...machine,
      mode: "selection_required",
      selection_required: true,
    },
    contextEnabled: false,
  });
  const button = screen.getByRole("button", {
    name: "Review shared legacy mode",
  });
  expect(button).toBeDisabled();
  fireEvent.click(
    screen.getByRole("checkbox", { name: /I explicitly choose shared legacy/ }),
  );
  fireEvent.click(button);
  await screen.findByText("Stored review review-1");
  expect(request.mock.calls[0]?.[1]?.body).toEqual({
    mode: "shared_legacy",
    policy_version: "machine-context-mode-v1",
    acknowledge_separation: true,
  });
});

it("disables an already reviewed separated commit when live context support disappears", async () => {
  const view = show();
  await prepare();
  fireEvent.click(
    screen.getByRole("checkbox", { name: /I approve this exact/ }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
    ).toBeEnabled(),
  );
  view.rerender(
    <QueryClientProvider client={client}>
      <MachineContextReview
        machine={{
          ...machine,
          context_setup_enabled: false,
          separated: {
            available: false,
            landlock_abi: 6,
            reason: "separated_probe_failed",
          },
        }}
        agentId="agent"
        contextEnabled={false}
        disabled={false}
      />
    </QueryClientProvider>,
  );
  expect(
    screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
  ).toBeDisabled();
  expect(
    request.mock.calls.filter(([path]) => path.endsWith("/commit")),
  ).toHaveLength(0);
});

it.each(["missing", "digest", "generation"])(
  "requires rereview when the current hosted configuration changes (%s)",
  async (change) => {
    const view = show();
    await prepare();
    fireEvent.click(
      screen.getByRole("checkbox", { name: /I approve this exact/ }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
      ).toBeEnabled(),
    );
    const configuration =
      change === "missing"
        ? null
        : {
            ...machine.machine_configuration,
            ...(change === "digest"
              ? { digest: "b".repeat(64) }
              : { runtime_generation: 5 }),
          };
    view.rerender(
      <QueryClientProvider client={client}>
        <MachineContextReview
          machine={{ ...machine, machine_configuration: configuration }}
          agentId="agent"
          contextEnabled
          disabled={false}
        />
      </QueryClientProvider>,
    );
    expect(
      screen.getByRole("button", { name: "Approve and apply reviewed mode" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("checkbox", { name: /I approve this exact/ }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Prepare and review again" }),
    ).toBeVisible();
    expect(
      request.mock.calls.filter(([path]) => path.endsWith("/commit")),
    ).toHaveLength(0);
  },
);
