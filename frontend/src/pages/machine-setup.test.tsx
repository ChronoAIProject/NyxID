import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { MachinePairPage, MachineSetupPage } from "./machine-setup";
import { machineSetupCommand } from "@/schemas/machines";
const mock = vi.hoisted(() => ({
  issue: vi.fn(),
  create: vi.fn(),
  preview: vi.fn(),
  decide: vi.fn(),
  setup: undefined as unknown,
  search: {} as Record<string, string>,
}));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => mock.search,
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: () => <span>Personal account</span>,
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({
    data: { node_ws_url: "wss://nyxid.example/api/v1/nodes/ws" },
  }),
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [] } }),
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineSetup: () => ({ data: mock.setup }),
  useCreateMachineSetup: () => ({ mutateAsync: mock.create }),
  issueMachineSetup: mock.issue,
  useMachinePairPreview: () => ({ mutateAsync: mock.preview }),
  useMachinePairDecision: () => ({ mutateAsync: mock.decide }),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  mock.setup = undefined;
  mock.search = {};
});
const choices = {
  name: "coding",
  where: "docker" as const,
  capabilities: ["shell" as const, "files" as const],
  grant_to: null,
};
it("shows one reviewed command only on the private setup page and removes it when connected", async () => {
  const client = new QueryClient();
  mock.search = { setup: "intent" };
  mock.setup = { id: "intent", choices, status: "review" };
  const token = `nyx_nreg_${"a".repeat(64)}`;
  mock.issue.mockResolvedValue({ token });
  const view = render(
    <QueryClientProvider client={client}>
      <MachineSetupPage />
    </QueryClientProvider>,
  );
  expect(screen.getByText(/prompt injection is possible/)).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.getByLabelText("Machine name")).toHaveValue("coding"),
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Create my setup command" }),
  );
  await waitFor(() =>
    expect(mock.issue).toHaveBeenCalledWith("intent", choices),
  );
  await waitFor(() =>
    expect(screen.getByText(/docker run -d/)).toHaveTextContent(token),
  );
  expect(JSON.stringify(client.getQueryCache().getAll())).not.toContain(token);
  expect(screen.getByText(/Do not paste it into chat/)).toBeInTheDocument();
  mock.setup = {
    id: "intent",
    choices,
    status: "connected",
    machine: { shell: true, files: true },
  };
  view.rerender(
    <QueryClientProvider client={client}>
      <MachineSetupPage />
    </QueryClientProvider>,
  );
  await waitFor(() =>
    expect(screen.queryByText(/docker run -d/)).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("status")).toHaveTextContent("Connected");
});
it("pairing shows machine details and requires explicit recognition before approval", async () => {
  mock.search = { code: "ABCD-2345" };
  mock.preview.mockResolvedValue({
    hostname: "work-vm",
    os: "linux",
    ip: "203.0.113.7",
    choices,
  });
  render(<MachinePairPage />);
  fireEvent.click(screen.getByRole("button", { name: "Review" }));
  await screen.findByText("work-vm");
  expect(screen.getByText("203.0.113.7")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Approve pairing" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Approve pairing" }));
  await waitFor(() =>
    expect(mock.decide).toHaveBeenCalledWith({
      code: "ABCD-2345",
      approve: true,
    }),
  );
});
it("quotes setup credentials and URLs, installs the CLI if missing and preserves chosen capabilities", () => {
  const token = `nyx_nreg_${"b".repeat(64)}`;
  const host = machineSetupCommand(
    { ...choices, where: "vm" },
    token,
    "wss://example.test/'literal",
  );
  expect(host).toContain("command -v nyxid");
  expect(host).toContain('PATH="$HOME/.local/bin:$PATH"');
  expect(host).toContain("--shell --files");
  expect(host).not.toContain("--computer");
  expect(host).toContain("'\\''literal'");
  expect(() =>
    machineSetupCommand(choices, "not-a-token", "wss://example.test"),
  ).toThrow();
});
