import type { Page, Route } from "@playwright/test";
import type {
  RemoteMachine,
  RemoteMachineOperation,
} from "../src/schemas/machines";
import type { MachineGrant } from "../src/schemas/machine-grants";
import {
  remoteMachineFixture,
  remoteMachineOperationFixture,
  remoteMachinePaidQuoteFixture,
  remoteMachinePlatformFixture,
} from "../src/test/remote-machine-fixture";
import { billingWallet } from "../src/test/billing-fixture";
import { assistantAgentListSchema } from "../src/schemas/assistant-nyxagent";
import type { ApiKey } from "../src/types/api";

export const machineId = "11111111-1111-4111-8111-111111111111";
export const otherMachineId = "22222222-2222-4222-8222-222222222222";
export const operationId = "delete-original";
const createdAt = "2026-10-14T01:00:00Z";
export const emptyCapabilities = {
  shell: false,
  files: false,
  browser: false,
  computer: false,
  developer_browser: false,
};
export const agent = {
  id: "agent-research",
  name: "Researcher",
  display_name: "Fixture researcher",
  owner_id: "test-user",
  owner_kind: "person",
  kind: "specialist",
  status: "idle",
  can_maintain: true,
  services: ["preserved-service"],
  account_read: true,
  machines: [otherMachineId],
  logins: ["agent-login-preserve"],
  guest_access: { "preserved-service": "read" },
  created_at: createdAt,
  last_active_at: createdAt,
};

export const sourceKey: ApiKey = {
  id: "parent-key",
  name: "Fixture parent Key",
  description: null,
  key_prefix: "fixture",
  scopes: "proxy:read proxy:write",
  purpose: "general",
  state_version: 3,
  created_at: createdAt,
  last_used_at: null,
  expires_at: null,
  is_active: true,
  allowed_node_ids: [machineId, otherMachineId],
  allowed_service_ids: ["33333333-3333-4333-8333-333333333333"],
  allow_all_nodes: false,
  allow_all_services: false,
  allowed_nodes: [],
  allowed_services: [],
  platform: null,
  callback_url: null,
  rate_limit_per_second: null,
  rate_limit_burst: null,
  bindings_count: 0,
};

export function deleteOperation(
  patch: Partial<RemoteMachineOperation> = {},
): RemoteMachineOperation {
  return {
    ...remoteMachineOperationFixture,
    id: operationId,
    machine_id: machineId,
    action: "delete",
    state: "reconciling",
    cleanup_state: "pending",
    retry_count: 153,
    delete_retry_attempts: 10,
    delete_retry_limit: 10,
    delete_retry_stopped: true,
    deadline_at: "2026-10-15T01:00:00Z",
    error_code: "delete_kubernetes_access_denied",
    error_message: "Platform access denied; operator intervention required.",
    ...patch,
  };
}

export function sourceGrant(multiple: boolean): MachineGrant {
  const assignment = {
    capabilities: { ...emptyCapabilities, shell: true, files: true },
    mode: "shared_legacy",
    revision: 4,
    saved_login_ids: [],
    file_roots: ["/workspace"],
    service_ids: ["33333333-3333-4333-8333-333333333333"],
  };
  return {
    id: "source-machine-grant",
    source: {
      kind: "api_key",
      api_key_id: "parent-key",
      api_key_state_version_at_approval: 3,
    },
    revision: 7,
    policy_revision: 1,
    status: "active",
    expires_at: "2099-01-01T00:00:00Z",
    created_at: createdAt,
    updated_at: createdAt,
    revoked_at: null,
    revoked_reason: null,
    assignments: {
      [machineId]: assignment,
      ...(multiple
        ? {
            [otherMachineId]: {
              ...assignment,
              file_roots: ["/other-workspace"],
            },
          }
        : {}),
    },
  };
}

export interface ApiWrite {
  path: string;
  method: string;
  body: unknown;
  key?: string;
}
export interface MachineMock {
  machine: RemoteMachine;
  operation: RemoteMachineOperation | null;
  grant: MachineGrant | null;
  assigned: boolean;
  writes: ApiWrite[];
  reads: string[];
  unknown: string[];
  agents: boolean;
  capabilitiesEditable: boolean;
  admissions: boolean;
  listed: boolean;
  detailStatus: number;
}

export async function fixtureError(
  route: Route,
  status: number,
  message: string,
) {
  await route.fulfill({
    status,
    json: { error: "fixture_error", error_code: status, message },
  });
}

/** One fail-closed router: unknown API paths are fulfilled locally, never continued. */
export async function installMachineMock(
  page: Page,
  options: Partial<MachineMock> = {},
) {
  const state: MachineMock = {
    machine: {
      ...remoteMachineFixture,
      node_id: machineId,
      name: "fixture-machine",
      resource_state: "ready",
      authorization_state: "active",
      active_operation_id: null,
      revision: 58,
    },
    operation: null,
    grant: null,
    assigned: false,
    writes: [],
    reads: [],
    unknown: [],
    agents: false,
    capabilitiesEditable: true,
    admissions: true,
    listed: true,
    detailStatus: 200,
    ...options,
  };
  const node = () => ({
    id: machineId,
    name: state.machine.name,
    owner: { kind: "user", id: "test-user", display_name: "Fixture owner" },
    status: "online",
    is_connected: true,
    metadata: { provisioning_source: "remote_machine" },
    machine_confirm: "changes",
    machine: {
      shell: true,
      files: true,
      browser: true,
      computer: true,
      computer_ready: true,
      isolated: true,
    },
  });
  await page.routeWebSocket("**/*", (socket) => {
    // Keep intercepted connections local. Vite needs its connected handshake;
    // immediately closing its socket causes a development-client error loop.
    if (new URL(socket.url()).pathname === "/")
      socket.send(JSON.stringify({ type: "connected" }));
    socket.onMessage(() => {});
  });
  await page.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const local = url.origin === "http://localhost:4628";
    if (!local) return route.abort("blockedbyclient");
    const api = url.pathname.startsWith("/api/");
    if (!api) {
      // Only app assets/documents can leave this router. OAuth, MCP, arbitrary
      // fetches and other origins are denied, including localhost on other ports.
      if (
        local &&
        ["document", "script", "stylesheet", "image", "font", "other"].includes(
          request.resourceType(),
        ) &&
        request.method() === "GET" &&
        !/^\/(oauth|mcp|health|\.well-known)(\/|$)/.test(url.pathname)
      )
        return route.continue();
      return route.abort("blockedbyclient");
    }
    const path = url.pathname.replace(/^\/api\/v1/, "");
    const method = request.method();
    if (method === "GET") state.reads.push(path + url.search);
    else
      state.writes.push({
        path,
        method,
        body: request.postData() ? request.postDataJSON() : null,
        key: request.headers()["idempotency-key"],
      });
    let body: unknown;
    if (method !== "GET") {
      state.unknown.push(`${method} ${path}`);
      return fixtureError(
        route,
        404,
        "Unconfigured fixture mutation blocked locally",
      );
    }
    if (path === "/users/me")
      body = {
        id: "test-user",
        email: "fixture@example.test",
        display_name: "Fixture owner",
        is_admin: false,
        is_active: true,
        email_verified: true,
        onboarding_completed: true,
        created_at: createdAt,
        capabilities: {
          billing_available: true,
          enabled_features: [
            "experimental:ai-assistant",
            "assistant:machine-capabilities",
            ...(state.admissions ? ["experimental:remote-machines"] : []),
          ],
        },
      };
    else if (path === "/remote-machines")
      body = {
        machines: state.listed ? [state.machine] : [],
        next_cursor: null,
      };
    else if (path === `/remote-machines/${machineId}`) {
      if (state.detailStatus !== 200)
        return fixtureError(
          route,
          state.detailStatus,
          state.detailStatus === 403
            ? "This machine is unavailable to this account."
            : "Fixture machine read failed.",
        );
      body = { machine: state.machine };
    } else if (
      path === `/remote-machine-operations/${operationId}` &&
      state.operation
    )
      body = { operation: state.operation };
    else if (path === `/remote-machines/${machineId}/operations`)
      body = {
        operations: state.operation ? [state.operation] : [],
        next_cursor: null,
      };
    else if (path === "/remote-machine-entitlement")
      body = {
        eligible: true,
        free_slot_available: false,
        machine_id: machineId,
        reason: null,
        policy_version: "remote-machine-mvp",
        policy: remoteMachinePaidQuoteFixture.policy,
        platform: remoteMachinePlatformFixture,
      };
    else if (path === "/nodes")
      body = {
        nodes: state.machine.resource_state === "deleted" ? [] : [node()],
      };
    else if (path === `/nodes/${machineId}`) body = node();
    else if (path === `/remote-machines/${machineId}/usage`)
      body = {
        available: false,
        stale: true,
        received_at: null,
        workspace: null,
        temporary: null,
        disk_full: false,
        write_limit_bytes: 1048576,
        recovery: "Remove unneeded files through an authorized client.",
      };
    else if (path === `/remote-machines/${machineId}/events`)
      body = { events: [], next_cursor: null };
    else if (path === `/remote-machines/${machineId}/grants`)
      body = {
        external_grants: state.grant?.assignments[machineId]
          ? [state.grant]
          : [],
        next_cursor: null,
      };
    else if (path === `/remote-machines/${machineId}/jobs`)
      body = {
        jobs: [],
        next_cursor: null,
        has_more: false,
        output_persisted: false,
      };
    else if (path === "/machine-confirmations")
      body = { confirmations: [], next_cursor: null };
    else if (path === "/assistant/nyxagent/agents")
      body = assistantAgentListSchema.parse({
        limits: {
          skip_destructive_confirmation: false,
          max_live_subagents: 2,
          max_concurrent_subagent_turns: 1,
          max_live_subagents_limit: 8,
          max_concurrent_subagent_turns_limit: 4,
        },
        agents: state.agents
          ? [
              {
                ...agent,
                machines: state.assigned
                  ? [otherMachineId, machineId]
                  : [otherMachineId],
              },
            ]
          : [],
      });
    else if (path === `/assistant/nyxagent/agents/${agent.id}`)
      body = {
        agent: {
          ...agent,
          machines: state.assigned
            ? [otherMachineId, machineId]
            : [otherMachineId],
        },
        memory: [],
        threads: [],
      };
    else if (path === `/assistant/nyxagent/agents/${agent.id}/machines`)
      body = state.assigned
        ? [
            {
              node_id: machineId,
              name: state.machine.name,
              revision: 8,
              protocol_v2: true,
              can_edit: state.capabilitiesEditable,
              legacy: false,
              mode: "shared_legacy",
              capabilities: emptyCapabilities,
              ceiling: {
                shell: true,
                files: true,
                browser: true,
                computer: true,
                developer_browser: true,
              },
              saved_login_ids: ["saved-login-preserve"],
            },
          ]
        : [];
    else if (path === "/api-keys/parent-key") body = sourceKey;
    else if (path === "/api-keys/parent-key/machine-grant")
      body = {
        grant: state.grant,
        source_valid: true,
        source_validity_reason: null,
        grant_valid: state.grant?.status === "active",
        grant_validity_reason:
          state.grant?.status === "active"
            ? null
            : state.grant
              ? "revoked"
              : "missing",
        source_and_grant_valid: state.grant?.status === "active",
      };
    else if (path === "/billing/wallet")
      body = billingWallet({
        owner_id: "test-user",
        available: "12",
        available_credits: 12,
      });
    else if (path === "/public/config")
      body = { mcp_url: "https://fixture.example.test/mcp" };
    else if (path === "/orgs") body = { organizations: [], orgs: [] };
    else if (path === "/api-keys") body = { keys: [] };
    else {
      state.unknown.push(`${method} ${path}`);
      return fixtureError(
        route,
        404,
        "Fixture endpoint intentionally unavailable",
      );
    }
    return route.fulfill({ json: body });
  });
  return state;
}
