/** Executed by the Rust replica-set test, with its isolated production router.
 * A normal frontend-only run explicitly skips this cross-process check. */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, onTestFailed, vi } from "vitest";
import type { ReactNode } from "react";
import type { User } from "@/types/api";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import {
  machineDraftKey,
  useMachineSettingsDraftStore,
} from "@/stores/machine-settings-draft-store";
import { MachineSettings } from "./machine-settings";
import type { NodeInfo } from "@/types/nodes";
import { machineAccessSchema } from "@/schemas/machine-access";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/components/assistant/nyxbot-agent-forms", () => ({
  NewAgentDialog: () => null,
}));

const origin = process.env.NYXID_F04_TEST_ORIGIN;
const owner = process.env.NYXID_F04_TEST_OWNER;
const machine = process.env.NYXID_F04_TEST_MACHINE;
const token = process.env.NYXID_F04_TEST_TOKEN;
const partial = process.env.NYXID_F04_TEST_PARTIAL === "true";
const originalAuth = useAuthStore.getState();
const client = new QueryClient({
  defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
});
afterEach(() => {
  cleanup();
  client.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  useMachineSettingsDraftStore.setState({ drafts: {} });
  useAuthStore.setState(originalAuth);
});

it.skipIf(!origin)(
  "real membership hook survives a server-only disable and retains partial-save retries",
  async () => {
    expect(owner && machine && token).toBeTruthy();
    onTestFailed(({ task }) => {
      for (const error of task.result?.errors ?? [])
        process.stderr.write(
          `${error.message.replace(token!, "[REDACTED]")}\n`,
        );
    });
    useAuthStore.setState({
      user: {
        id: owner,
        capabilities: { enabled_features: [FEATURE_FLAG.MACHINE_CAPABILITIES] },
      } as unknown as User,
    });
    const writes: unknown[] = [];
    const endpoints: string[] = [];
    // This boundary forwards bytes to the production Rust router, with real auth,
    // DTO parsing, flag reads and transaction writes. No synthetic success response.
    globalThis.__nyxidAssistantHttpMock = ({ endpoint, init }) => {
      if (init.method === "PUT") {
        writes.push(JSON.parse(String(init.body)));
        endpoints.push(endpoint);
      }
      return fetch(`${origin}/api/v1${endpoint}`, {
        ...init,
        headers: { ...init.headers, Authorization: `Bearer ${token!}` },
      });
    };
    const node: NodeInfo = {
      id: machine!,
      name: "Live isolated machine",
      is_connected: true,
      owner: { id: owner!, kind: "user", display_name: "Owner" },
      status: "online",
      last_heartbeat_at: null,
      connected_at: null,
      metadata: null,
      metrics: null,
      binding_count: 0,
      created_at: "2026-10-26T00:00:00Z",
      machine: {
        version: 1,
        shell: true,
        files: true,
        computer: false,
        roots: [],
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
    render(
      <QueryClientProvider client={client}>
        <MachineSettings node={node} canManage />
      </QueryClientProvider>,
    );
    fireEvent.click(
      await screen.findByRole("checkbox", { name: "Live worker one" }),
    );
    if (partial)
      fireEvent.click(
        screen.getByRole("checkbox", { name: "Live worker two" }),
      );
    fireEvent.click(
      screen.getByRole("button", { name: "Save machine settings" }),
    );
    if (partial) {
      await screen.findByText(/Live worker two needs another try/);
      expect(writes).toHaveLength(2);
      fireEvent.click(
        screen.getByRole("button", { name: "Save machine settings" }),
      );
    }
    await waitFor(
      () => {
        const draft =
          useMachineSettingsDraftStore.getState().drafts[
            machineDraftKey(owner, owner!, machine!)
          ];
        expect(draft?.saving).toBe(false);
        expect(draft?.members).toEqual({});
        expect(draft?.errors).toEqual([]);
      },
      { timeout: 10_000 },
    );
    expect(
      screen.getByRole("button", { name: "Save machine settings" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("checkbox", { name: "Live worker one" }),
    ).toBeChecked();
    if (partial)
      expect(
        screen.getByRole("checkbox", { name: "Live worker two" }),
      ).toBeChecked();
    expect(writes).toHaveLength(partial ? 3 : 1);
    for (const write of writes)
      expect(write).toEqual({
        machine_membership: { node_id: machine, assigned: true },
      });
    const accessResponse = await fetch(
      `${origin}/api/v1${endpoints[0]!.replace(/\/grants$/, "/machines")}`,
      { headers: { Authorization: `Bearer ${token!}` } },
    );
    expect(accessResponse.ok).toBe(true);
    const access = machineAccessSchema
      .array()
      .parse(await accessResponse.json());
    expect(access.find((option) => option.node_id === machine)).toMatchObject({
      membership: true,
      capability_edit_enabled: false,
    });
    expect(
      useAuthStore.getState().user?.capabilities?.enabled_features,
    ).toContain(FEATURE_FLAG.MACHINE_CAPABILITIES);
  },
  20_000,
);
