import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { useMachineSettingsDraftStore } from "@/stores/machine-settings-draft-store";
import type { NodeInfo } from "@/types/nodes";
import type { User } from "@/types/api";
import { MachineSettings } from "./machine-settings";
import { MachineSettingsEditor } from "./machine-settings-editor";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, search }: { children: ReactNode; to: string; search?: Record<string, string> }) => <a href={`${to}?${new URLSearchParams(search)}`}>{children}</a>,
}));
vi.mock("@/components/assistant/nyxbot-agent-forms", () => ({
  NewAgentDialog: () => null,
}));
vi.mock("@/hooks/use-machines", () => ({
  useMachineSettings: () => ({ mutateAsync: vi.fn() }),
  useMachineUpdatePolicy: () => ({ mutateAsync: vi.fn() }),
}));

const node: NodeInfo = {
  id: "machine",
  name: "Self-hosted machine",
  is_connected: true,
  owner: { id: "owner", kind: "user", display_name: "Owner" },
  status: "online",
  last_heartbeat_at: null,
  connected_at: null,
  metadata: null,
  metrics: null,
  binding_count: 0,
  created_at: "2026-10-26T00:00:00Z",
  machine_confirm: "none",
  machine: {
    version: 1,
    shell: true,
    files: true,
    computer: false,
    roots: ["/workspace"],
    os: "linux",
    arch: "arm64",
    computer_mode: "standard",
    cua_version: null,
    computer_tools: [],
    computer_ready: false,
    browser_isolated: false,
    saved_login_ready: false,
  },
};
const auth = useAuthStore.getState();
let client: QueryClient;
let machines: string[];
let secondMachines: string[];
let writes: { endpoint: string; body: Record<string, unknown> }[];
function flag(enabled: boolean) {
  useAuthStore.setState({
    user: {
      id: "owner",
      capabilities: {
        enabled_features: enabled ? [FEATURE_FLAG.MACHINE_CAPABILITIES] : [],
      },
    } as unknown as User,
  });
}
beforeEach(() => {
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  machines = ["other"];
  secondMachines = ["other"];
  writes = [];
  flag(false);
  // Keep the real list parser, grants hook, owner guard and JSON transport.
  // Only the HTTP boundary is synthetic; backend transactional tests cover its consumer.
  globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
    if (endpoint.startsWith("/assistant/nyxagent/agents?"))
      return Response.json({
        agents: [
          {
            id: "specialist",
            name: "Coder",
            kind: "specialist",
            status: "idle",
            owner_id: "owner",
            can_maintain: true,
            machines,
            services: ["github"],
            account_read: true,
            logins: ["login"],
            guest_access: { github: "read" },
            created_at: "2026-10-26T00:00:00Z",
            last_active_at: "2026-10-26T00:00:00Z",
          },
        ],
        limits: {
          skip_destructive_confirmation: false,
          max_live_subagents: 5,
          max_concurrent_subagent_turns: 2,
          max_live_subagents_limit: 5,
          max_concurrent_subagent_turns_limit: 2,
        },
      });
    if (
      /^\/assistant\/nyxagent\/agents\/(specialist|second)\/grants$/.test(
        endpoint,
      ) &&
      init.method === "PUT"
    ) {
      const body = JSON.parse(init.body as string) as Record<string, unknown>;
      writes.push({ endpoint, body });
      const member = body.machine_membership as {
        node_id: string;
        assigned: boolean;
      };
      const update = (ids: string[]) =>
        member.assigned
          ? [...new Set([...ids, member.node_id])]
          : ids.filter((id) => id !== member.node_id);
      if (endpoint.includes("/specialist/")) machines = update(machines);
      else secondMachines = update(secondMachines);
      return Response.json({});
    }
    throw new Error(`Unexpected request: ${init.method ?? "GET"} ${endpoint}`);
  };
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useMachineSettingsDraftStore.setState({ drafts: {} });
  useAuthStore.setState(auth);
  vi.clearAllMocks();
});
function mount(
  children: ReactNode = <MachineSettings node={node} canManage />,
) {
  return render(
    <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  );
}

it.each([false, true])(
  "sends an isolated membership request with editing %s",
  async (enabled) => {
    flag(enabled);
    mount();
    fireEvent.click(await screen.findByRole("checkbox", { name: "Coder" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Save machine settings" }),
    );
    await waitFor(() => expect(writes).toHaveLength(1));
    expect(writes[0]).toEqual({
      endpoint: "/assistant/nyxagent/agents/specialist/grants",
      body: { machine_membership: { node_id: "machine", assigned: true } },
    });
  },
);

it("keeps existing self-hosted assignments reviewable and removable with editing off", async () => {
  machines.push("machine");
  mount();
  expect(await screen.findByRole("checkbox", { name: "Coder" })).toBeEnabled();
  expect(
    screen.getByRole("link", { name: "Review Coder separation" }),
  ).toBeEnabled();
  fireEvent.click(screen.getByRole("checkbox", { name: "Coder" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  fireEvent.click(
    await screen.findByRole("button", { name: "Save and remove access" }),
  );
  await waitFor(() => expect(writes).toHaveLength(1));
  expect(writes[0]!.body).toEqual({
    machine_membership: { node_id: "machine", assigned: false },
  });
});

it("retains a pending addition across close and saves membership after capability editing turns off", async () => {
  flag(true);
  const view = mount();
  fireEvent.click(await screen.findByRole("checkbox", { name: "Coder" }));
  view.unmount();
  flag(false);
  mount();
  expect(await screen.findByRole("checkbox", { name: "Coder" })).toBeChecked();
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await waitFor(() => expect(writes).toHaveLength(1));
  expect(writes[0]!.body).toEqual({
    machine_membership: { node_id: "machine", assigned: true },
  });
  expect(screen.getByRole("checkbox", { name: "Coder" })).toBeChecked();
});

it.each([false, true])(
  "keeps hosted membership-only writes available with capability editing %s",
  async (enabled) => {
    flag(enabled);
    mount(
      <MachineSettingsEditor
        machineId="machine"
        ownerId="owner"
        name="Hosted"
        editable
        connected
        readOnlyPolicy
      />,
    );
    fireEvent.click(await screen.findByRole("checkbox", { name: "Coder" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Save machine settings" }),
    );
    await waitFor(() => expect(writes).toHaveLength(1));
    expect(writes[0]!.body).toEqual({
      machine_membership: { node_id: "machine", assigned: true },
    });
    expect(
      screen.getByText(/New assignments start with no tools/),
    ).toBeInTheDocument();
  },
);

it("retains partial successes and retries membership with a changed flag without a replacement fallback", async () => {
  flag(true);
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = async (request) => {
    const response = await original(request);
    if (request.endpoint.startsWith("/assistant/nyxagent/agents?")) {
      const body = await response!.json();
      body.agents.push({
        ...body.agents[0],
        id: "second",
        name: "Second",
        machines: secondMachines,
      });
      return Response.json(body);
    }
    if (request.init.method === "PUT") {
      flag(false);
      if (request.endpoint.includes("/second/") && writes.length === 2) {
        secondMachines = ["other"];
        return Response.json(
          { error: "unavailable", error_code: 503, message: "Retry" },
          { status: 503 },
        );
      }
    }
    return response;
  };
  mount();
  fireEvent.click(await screen.findByRole("checkbox", { name: "Coder" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Second" }));
  await act(async () => {
    fireEvent.click(
      screen.getByRole("button", { name: "Save machine settings" }),
    );
  });
  await screen.findByText(/Second needs another try/);
  expect(writes).toHaveLength(2);
  expect(screen.getByRole("checkbox", { name: "Coder" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Second" })).toBeChecked();
  await act(async () => {
    flag(true);
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await waitFor(() => expect(writes).toHaveLength(3));
  expect(writes.map((write) => write.endpoint)).toEqual([
    "/assistant/nyxagent/agents/specialist/grants",
    "/assistant/nyxagent/agents/second/grants",
    "/assistant/nyxagent/agents/second/grants",
  ]);
});

it("keeps errors and drafts when a server rejects the new form and never retries legacy semantics", async () => {
  const original = globalThis.__nyxidAssistantHttpMock!;
  globalThis.__nyxidAssistantHttpMock = async (request) => {
    if (request.init.method === "PUT") {
      writes.push({endpoint:request.endpoint,body:JSON.parse(String(request.init.body)) as Record<string,unknown>});
      return Response.json({error:"validation_error",error_code:1000,message:"Unsupported payload"},{status:422});
    }
    return original(request);
  };
  mount();
  fireEvent.click(await screen.findByRole("checkbox", { name: "Coder" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Save machine settings" }),
  );
  await screen.findByText(/Coder needs another try/);
  expect(writes).toHaveLength(1);
  expect(writes[0]!.body).toEqual({
    machine_membership: { node_id: "machine", assigned: true },
  });
  expect(screen.getByRole("checkbox", { name: "Coder" })).toBeChecked();
});
