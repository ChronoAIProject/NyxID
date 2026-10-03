import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { MachineActivity } from "./machine-activity";
import { MachineSharingBadge } from "./machine-isolation";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import { api } from "@/lib/api-client";
vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn() } }));
const agent = "11111111-1111-4111-8111-111111111111";
const page = {
  machine_name: "Work MacBook Air",
  agents: [
    {
      id: agent,
      name: "researcher",
      display_name: "Research helper",
      kind: "specialist",
    },
  ],
  entries: [
    {
      id: "event",
      agent_id: agent,
      actor_id: null,
      action: "file.read",
      outcome: "completed",
      operation_id: "operation",
      activity_id: "activity",
      job_id: null,
      exit_code: null,
      duration_ms: 1,
      created_at: "2026-10-03T00:00:00Z",
    },
  ],
  next_cursor: "next",
};
const clients: QueryClient[] = [];
function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  clients.push(client);
  return render(
    <QueryClientProvider client={client}>
      <OverlayLayer layer={90}>
        <MachineActivity nodeId="machine" />
      </OverlayLayer>
    </QueryClientProvider>,
  );
}
afterEach(() => {
  cleanup();
  clients.splice(0).forEach((c) => c.clear());
  vi.clearAllMocks();
});
it("labels shared sessions independently of command isolation", () => {
  render(<MachineSharingBadge />);
  expect(screen.getByText("Shared workspace and browser")).toHaveAttribute(
    "title",
    expect.stringContaining("authenticated browser sessions"),
  );
});
it("shows names and avatars, keeps IDs in disclosure, and filters bounded pages via the shortcut", async () => {
  vi.mocked(api.get).mockResolvedValue(page);
  mount();
  expect(await screen.findByText("Work MacBook Air")).toBeInTheDocument();
  expect(screen.getByText("Research helper (@researcher)")).toBeInTheDocument();
  expect(screen.getByText("RH")).toHaveAttribute("aria-hidden", "true");
  const rawId = screen.getByText(agent);
  expect(rawId.closest("details")).not.toHaveAttribute("open");
  expect(rawId).not.toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Show this agent" }));
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith(
      `/machines/machine/activity?limit=50&agent_id=${agent}`,
    ),
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Next" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith(
      `/machines/machine/activity?limit=50&agent_id=${agent}&before=next`,
    ),
  );
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
});
it("offers a named agent picker above the enclosing overlay and resets pagination on selection", async () => {
  const user = userEvent.setup();
  vi.mocked(api.get).mockResolvedValue(page);
  mount();
  await screen.findByText("Work MacBook Air");
  await user.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith(
      "/machines/machine/activity?limit=50&before=next",
    ),
  );
  await user.click(screen.getByRole("combobox", { name: "Filter by agent" }));
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  const list = screen.getByRole("listbox");
  expect(list).toHaveStyle({ zIndex: "100" });
  await user.click(
    within(list).getByRole("option", { name: "Research helper (@researcher)" }),
  );
  await waitFor(() =>
    expect(api.get).toHaveBeenLastCalledWith(
      `/machines/machine/activity?limit=50&agent_id=${agent}`,
    ),
  );
  expect(screen.getByRole("button", { name: "Previous" })).toBeDisabled();
});
it("labels and filters unattributed rows as Unknown (older node)", async () => {
  const user = userEvent.setup();
  vi.mocked(api.get).mockResolvedValue({
    ...page,
    entries: [{ ...page.entries[0], agent_id: null }],
  });
  mount();
  expect(await screen.findByText("Unknown (older node)")).toBeInTheDocument();
  await user.click(screen.getByRole("combobox", { name: "Filter by agent" }));
  await user.click(
    screen.getByRole("option", { name: "Unknown (older node)" }),
  );
  await waitFor(() =>
    expect(api.get).toHaveBeenCalledWith(
      "/machines/machine/activity?limit=50&unattributed=true",
    ),
  );
});
it("does not expose an unavailable agent ID as its label and falls back for a missing machine name", async () => {
  vi.mocked(api.get).mockResolvedValue({
    ...page,
    agents: [],
    machine_name: null,
  });
  mount();
  expect(await screen.findByText("Agent unavailable")).toBeInTheDocument();
  expect(screen.getByText("Machine machine")).toBeInTheDocument();
  expect(screen.getByText(agent)).not.toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Show this agent" }),
  ).not.toBeInTheDocument();
});
