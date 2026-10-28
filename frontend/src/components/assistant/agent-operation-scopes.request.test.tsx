import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentOperationScopes } from "./agent-operation-scopes";
import { OperationScopeCard } from "@/components/dashboard/api-key-detail/operation-scope-card";
import { useAuthStore } from "@/stores/auth-store";
import type { AgentServiceOperations, OperationSelection } from "@/schemas/agent-operation-scopes";
import type { User } from "@/types/api";

// Exercise both editors' real hooks, schemas and HTTP clients with a local fixture.
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));

let path: string;
const operations = ["drive-list", "drive-get", "docs-get", "sheets-get", "slides-get"];
let service: AgentServiceOperations;
let saves: OperationSelection[];
let ready: boolean;
let validationFailure: boolean;
let client: QueryClient;

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  useAuthStore.setState({ user: { id: "permissions-owner" } as User });
  ready = false;
  validationFailure = false;
  saves = [];
  service = {
    service_id: "workspace", service_slug: "google-workspace", service_name: "Google Workspace",
    revision: 4, all_operations: false, allows_explicit_rules: false, endpoint_ids: [], rules: [],
    operations: [...operations, "drive-delete"].map((id) => ({
      endpoint_id: id, method: id === "drive-delete" ? "DELETE" : "GET", path: `/${id}`,
      summary: id, read_only: id !== "drive-delete", changes_existing: id === "drive-delete",
    })),
  };
  client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } } });
  vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => {
    if (url === `${path}/workspace` && init?.method === "PUT") {
      const selection = JSON.parse(String(init.body)) as OperationSelection;
      saves.push(selection);
      if (validationFailure) return new Response(JSON.stringify({
        error: "conflict", error_code: 1005,
        message: "Operation scopes changed. Reload and try again.",
      }), { status: 409, headers: { "X-NyxID-Trace-Id": "permissions_trace-2007" } });
      if (!ready) return new Response(JSON.stringify({
        error: "security_readiness_unavailable", error_code: 12900, message: "private diagnostic",
      }), { status: 503, headers: { "X-NyxID-Trace-Id": "permissions_trace-2007" } });
      service = { ...service, revision: 5, endpoint_ids: selection.endpoint_ids };
      return Response.json({ revision: 5 });
    }
    if (url === path) return Response.json([service]);
    throw new Error("Unexpected permissions fixture route");
  }));
});

it.each(["specialist", "agent-key"] as const)("%s preserves non-readiness errors without changing the draft or duplicating a trace", async (editor) => {
  validationFailure = true;
  path = editor === "specialist"
    ? "/api/v1/assistant/nyxagent/agents/permissions-agent/operations"
    : "/api/v1/api-keys/permissions-key/operations";
  render(<QueryClientProvider client={client}>{editor === "specialist"
    ? <AgentOperationScopes agentId="permissions-agent" />
    : <OperationScopeCard keyId="permissions-key" canWrite />}</QueryClientProvider>);
  fireEvent.click(await screen.findByRole("button", { name: "Select all reads" }));
  fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toBe("Operation scopes changed. Reload and try again."
    + (editor === "specialist" ? "\nTrace ID: permissions_trace-2007" : ""));
  expect(saves).toEqual([{ expected_revision: 4, all_operations: false, endpoint_ids: operations, rules: [] }]);
  for (const id of operations) expect(screen.getByRole("checkbox", { name: `GET /${id}` })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "DELETE /drive-delete" })).not.toBeChecked();
  await waitFor(() => expect(screen.getByRole("button", { name: "Save operations" })).toBeEnabled());
});

afterEach(() => {
  cleanup();
  client.clear();
  useAuthStore.setState({ user: null });
  vi.unstubAllGlobals();
});

it.each(["specialist", "agent-key"] as const)("%s retains five dirty selections after readiness rejection and saves the same draft on retry", async (editor) => {
  path = editor === "specialist"
    ? "/api/v1/assistant/nyxagent/agents/permissions-agent/operations"
    : "/api/v1/api-keys/permissions-key/operations";
  render(<QueryClientProvider client={client}>{editor === "specialist"
    ? <AgentOperationScopes agentId="permissions-agent" />
    : <OperationScopeCard keyId="permissions-key" canWrite />}</QueryClientProvider>);
  await screen.findByRole("button", { name: "Select all reads" });
  fireEvent.click(screen.getByRole("button", { name: "Select all reads" }));
  fireEvent.click(screen.getByRole("button", { name: "Save operations" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("An administrator must complete security rollout readiness");
  expect(alert).toHaveTextContent("proposed changes are still unsaved");
  if (editor === "specialist") expect(alert).toHaveTextContent("Trace ID: permissions_trace-2007");
  expect(alert).not.toHaveTextContent("private diagnostic");
  for (const id of operations) expect(screen.getByRole("checkbox", { name: `GET /${id}` })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "DELETE /drive-delete" })).not.toBeChecked();
  const save = screen.getByRole("button", { name: "Save operations" });
  await waitFor(() => expect(save).toBeEnabled());
  expect(saves).toEqual([{ expected_revision: 4, all_operations: false, endpoint_ids: operations, rules: [] }]);
  ready = true;
  fireEvent.click(save);
  await waitFor(() => expect(saves).toHaveLength(2));
  expect(saves[1]).toEqual(saves[0]);
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  await waitFor(() => expect(screen.getByRole("button", { name: "Save operations" })).toBeDisabled());
  expect(screen.getByText("Revision 5")).toBeInTheDocument();
  for (const id of operations) expect(screen.getByRole("checkbox", { name: `GET /${id}` })).toBeChecked();
});
