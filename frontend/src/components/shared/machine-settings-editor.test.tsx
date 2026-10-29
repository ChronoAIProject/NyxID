import {
  cleanup,
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { MachineSettingsEditor } from "./machine-settings-editor";
import { useAuthStore } from "@/stores/auth-store";
const state = vi.hoisted(() => ({
  save: vi.fn(),
  mode: "ready",
  refetch: vi.fn(),
  createIntent: undefined as string | undefined,
  agents: [
    {
      id: "one",
      name: "One",
      kind: "specialist",
      machines: ["other"],
      services: ["service"],
      account_read: true,
      logins: ["login"],
      guest_access: {},
    },
    {
      id: "two",
      name: "Two",
      kind: "specialist",
      machines: [],
      services: [],
      account_read: false,
    },
  ],
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({
    data:
      state.mode === "error" || state.mode === "loading"
        ? undefined
        : { agents: state.mode === "empty" ? [] : state.agents },
    isPending: state.mode === "loading",
    error: state.mode === "error" ? new Error("offline") : undefined,
    refetch: state.refetch,
  }),
  useSetNyxBotAgentGrants: () => ({ mutateAsync: state.save }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => false }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, search }: { children: React.ReactNode; to?: string; search?: Record<string, string> }) => (
    <a href={`${to ?? ""}?${new URLSearchParams(search)}`}>{children}</a>
  ),
}));
vi.mock("@/components/assistant/nyxbot-agent-forms", () => ({
  NewAgentDialog: ({ machineAssignment }: { machineAssignment?: string }) => {
    state.createIntent = machineAssignment;
    return <div>Nested Agent form</div>;
  },
}));
const auth = useAuthStore.getState();
beforeEach(() => {
  useAuthStore.setState({ user: { ...auth.user!, id: "editor-owner" } });
  state.agents[0]!.machines = ["other"];
  state.mode = "ready";
  state.save.mockResolvedValue({});
});
afterEach(() => {
  cleanup();
  useAuthStore.setState(auth);
  vi.clearAllMocks();
});
const props = {
  machineId: "machine",
  name: "Workspace",
  editable: true,
  connected: false,
  readOnlyPolicy: true,
};
it("restores owner and machine scoped selections across close and clears them on account switch", () => {
  const view = render(<MachineSettingsEditor {...props} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "One" }));
  view.unmount();
  const reopened = render(<MachineSettingsEditor {...props} />);
  expect(screen.getByRole("checkbox", { name: "One" })).toBeChecked();
  reopened.unmount();
  useAuthStore.setState({ user: { ...auth.user!, id: "other-owner" } });
  const switched = render(<MachineSettingsEditor {...props} />);
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
  switched.unmount();
  useAuthStore.setState({ user: { ...auth.user!, id: "editor-owner" } });
  render(<MachineSettingsEditor {...props} />);
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
});
it("keeps successful member saves, retries only failures, and preserves unrelated grants", async () => {
  state.save.mockImplementation(async ({ id }) => {
    if (id === "one") throw new Error("offline");
    return {};
  });
  render(<MachineSettingsEditor {...props} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "One" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Two" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await screen.findByText(/Some selections weren't saved/);
  expect(state.save).toHaveBeenCalledTimes(2);
  expect(state.save).toHaveBeenCalledWith(
    expect.objectContaining({
      id: "one",
      machine_membership: { node_id: "machine", assigned: true },
    }),
  );
  expect(screen.getByRole("checkbox", { name: "Two" })).toBeChecked();
  state.save.mockResolvedValue({});
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await waitFor(() => expect(state.save).toHaveBeenCalledTimes(3));
  expect(state.save.mock.calls[2]![0].id).toBe("one");
});
it("shows unknown policy and Version without update actions; blocks paused writes", () => {
  render(<MachineSettingsEditor {...props} editable={false} />);
  expect(screen.getByRole("region", { name: "Version" })).toHaveTextContent(
    "Not reported",
  );
  expect(
    screen.getByRole("region", { name: "Owner confirmation" }),
  ).toHaveTextContent("Not reported");
  expect(screen.queryByText("Version and updates")).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox", { name: "One" })).toBeDisabled();
});
it("keeps separate machine/owner drafts and discards only pending membership", async () => {
  const view = render(<MachineSettingsEditor {...props} ownerId="owner-a" />);
  fireEvent.click(screen.getByRole("checkbox", { name: "One" }));
  view.unmount();
  const other = render(<MachineSettingsEditor {...props} ownerId="owner-b" />);
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
  other.unmount();
  const otherMachine = render(
    <MachineSettingsEditor
      {...props}
      machineId="another-machine"
      ownerId="owner-a"
    />,
  );
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
  otherMachine.unmount();
  render(<MachineSettingsEditor {...props} ownerId="owner-a" />);
  expect(screen.getByRole("checkbox", { name: "One" })).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Discard changes" }));
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
  expect(state.save).not.toHaveBeenCalled();
});

it.each(["loading", "error", "empty"])(
  "shows the explicit Specialist %s state",
  async (mode) => {
    state.mode = mode;
    render(<MachineSettingsEditor {...props} />);
    if (mode === "loading")
      expect(screen.getByText("Loading specialists…")).toBeInTheDocument();
    else if (mode === "error") {
      fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      expect(state.refetch).toHaveBeenCalled();
    } else
      expect(
        screen.getByRole("button", { name: "Create a specialist" }),
      ).toBeEnabled();
    expect(state.save).not.toHaveBeenCalled();
  },
);

it("stops a multi-member save and drops old-account draft results after account switch", async () => {
  let release!: () => void;
  state.save.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
      }),
  );
  render(<MachineSettingsEditor {...props} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "One" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Two" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await waitFor(() => expect(state.save).toHaveBeenCalledTimes(1));
  await act(async () => {
    useAuthStore.setState({ user: { id: "another-account" } as never });
    release();
  });
  expect(state.save).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("checkbox", { name: "One" })).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Two" })).not.toBeChecked();
});
it("adopts newly read server policy without manufacturing unsaved changes", () => {
  const view = render(
    <MachineSettingsEditor
      {...props}
      readOnlyPolicy={false}
      confirmation="none"
    />,
  );
  view.rerender(
    <MachineSettingsEditor
      {...props}
      readOnlyPolicy={false}
      confirmation="all"
    />,
  );
  expect(
    screen.getByRole("combobox", { name: "Owner confirmation" }),
  ).toHaveTextContent("Confirm every operation");
  expect(
    screen.getByRole("button", { name: "Save machine settings" }),
  ).toBeDisabled();
});

it("passes membership-only intent to Settings-origin nested Agent creation", () => {
  state.mode = "empty";
  render(<MachineSettingsEditor {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Create a specialist" }));
  expect(screen.getByText("Nested Agent form")).toBeInTheDocument();
  expect(state.createIntent).toBe("membership_only");
});

it("routes Agent review to Assistant grants instead of opening an inline dialog", () => {
  state.agents[0]!.machines = ["machine"];
  render(<MachineSettingsEditor {...props} />);
  const review = screen.getByRole("link", { name: "Review One separation" });
  expect(review).toHaveAttribute(
    "href",
    "/assistant?agent=one&details=grants",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("saves machine policy while the Agent list is unavailable and keeps drafts", async () => {
  state.mode = "error";
  const onSavePolicy = vi.fn().mockResolvedValue(undefined);
  render(
    <MachineSettingsEditor
      {...props}
      readOnlyPolicy={false}
      confirmation="none"
      onSavePolicy={onSavePolicy}
    />,
  );
  fireEvent.click(screen.getByRole("combobox", { name: "Owner confirmation" }));
  fireEvent.click(screen.getByRole("option", { name: "Confirm every operation" }));
  expect(screen.getByRole("button", { name: "Save machine settings" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Save machine settings" }));
  await waitFor(() =>
    expect(onSavePolicy).toHaveBeenCalledWith(
      expect.objectContaining({ confirm: "all" }),
    ),
  );
});

it.each(["error", "loading"])("saves policy independently while retaining unsaved membership during Agent %s", async (mode) => {
  const onSavePolicy = vi.fn().mockResolvedValue(undefined);
  const view = render(<MachineSettingsEditor {...props} readOnlyPolicy={false} confirmation="none" onSavePolicy={onSavePolicy} />);
  fireEvent.click(screen.getByRole("checkbox", { name: "One" }));
  state.mode = mode;
  view.rerender(<MachineSettingsEditor {...props} readOnlyPolicy={false} confirmation="none" onSavePolicy={onSavePolicy} />);
  fireEvent.click(screen.getByRole("combobox", { name: "Owner confirmation" }));
  fireEvent.click(screen.getByRole("option", { name: "Confirm every operation" }));
  fireEvent.click(screen.getByRole("button", { name: "Save machine settings" }));
  await waitFor(() => expect(onSavePolicy).toHaveBeenCalledOnce());
  await screen.findByText(/Specialist selections remain unsaved/);
  expect(state.save).not.toHaveBeenCalled();
  state.mode = "ready";
  view.rerender(<MachineSettingsEditor {...props} readOnlyPolicy={false} confirmation="all" onSavePolicy={onSavePolicy} />);
  expect(screen.getByRole("checkbox", { name: "One" })).toBeChecked();
  expect(screen.queryByText("All changes saved")).not.toBeInTheDocument();
});
