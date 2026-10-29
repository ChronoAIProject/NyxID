import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { RemoteMachineActivity } from "./remote-machine-activity";
const calls = vi.hoisted(() => ({ tools: vi.fn(), events: vi.fn() }));
vi.mock("@/hooks/use-machine-activity", () => ({
  useMachineActivity: (id: string, agent: string, before?: string) => {
    calls.tools(id, agent, before);
    return {
      data: {
        entries: [],
        agents: [],
        next_cursor: before ? null : "tools-next",
      },
    };
  },
}));
vi.mock("@/hooks/use-machines", () => ({
  useRemoteMachineOperation: () => ({}),
  useRemoteMachineEvents: (id: string, cursor?: string) => {
    calls.events(id, cursor);
    return { data: { events: [], next_cursor: cursor ? null : "events-next" } };
  },
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
it("keeps Tool activity filters and cursors independent of Machine events", async () => {
  render(<RemoteMachineActivity machineId="paused-machine" />);
  await userEvent.click(
    screen.getByRole("combobox", { name: "Filter by agent" }),
  );
  await userEvent.click(
    screen.getByRole("option", { name: "Unknown (older node)" }),
  );
  fireEvent.click(
    within(screen.getByRole("tabpanel", { name: "Tool activity" })).getByRole(
      "button",
      { name: "Next" },
    ),
  );
  expect(calls.tools).toHaveBeenLastCalledWith(
    "paused-machine",
    "unknown",
    "tools-next",
  );
  await userEvent.click(screen.getByRole("tab", { name: "Machine events" }));
  fireEvent.click(
    within(screen.getByRole("tabpanel", { name: "Machine events" })).getByRole(
      "button",
      { name: "Older events" },
    ),
  );
  expect(calls.events).toHaveBeenLastCalledWith(
    "paused-machine",
    "events-next",
  );
  await userEvent.click(screen.getByRole("tab", { name: "Tool activity" }));
  expect(
    screen.getByRole("combobox", { name: "Filter by agent" }),
  ).toHaveTextContent("Unknown (older node)");
  expect(calls.tools).toHaveBeenLastCalledWith(
    "paused-machine",
    "unknown",
    "tools-next",
  );
  await userEvent.click(screen.getByRole("tab", { name: "Machine events" }));
  expect(
    within(screen.getByRole("tabpanel", { name: "Machine events" })).getByRole(
      "button",
      { name: "Newer events" },
    ),
  ).toBeEnabled();
});
it("honors a new activity deep link while the same machine drawer is mounted", () => {
  const view = render(
    <RemoteMachineActivity machineId="machine" initialTab="tools" />,
  );
  view.rerender(
    <RemoteMachineActivity machineId="machine" initialTab="events" />,
  );
  expect(screen.getByRole("tab", { name: "Machine events" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});
