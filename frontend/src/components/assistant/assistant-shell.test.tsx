import { useRef, type ReactNode } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAssistantLayoutStore } from "@/stores/assistant-layout-store";
import { AssistantShell } from "./assistant-shell";
import { ThreadHeader } from "./nyxbot-agent-panels";
import { AssistantWireLogAction } from "./assistant-wire-log-panel";
import { MockScenariosAction } from "./mock-scenarios-action";
import { useAuthStore } from "@/stores/auth-store";
import { CmaSessionContext } from "./cma-session-context";
import { cmaSession } from "@/lib/assistant/cma-session.test-fixtures";
import { initialCmaSession } from "@/lib/assistant/cma-session-api";
import type { CmaOperation } from "@/schemas/cma-operation";

const initialAuth = useAuthStore.getState();
const receipt: CmaOperation = {
  id: "watch",
  turn_id: "turn",
  service_id: cmaSession.connection.id,
  kind: "cma_provisioning",
  operation_id: null,
  agent_id: null,
  idempotency_key: "request",
  status: null,
  phase: null,
  state: "delivered",
  reason: "submission_uncertain",
  monitor_name: "NyxBot",
  created_at: "2026-11-10T10:00:00Z",
  checked_at: null,
};

const viewport = vi.hoisted(() => ({ desktop: true }));
vi.mock("@/hooks/use-media-query", () => ({ useMediaQuery: () => viewport.desktop }));
vi.mock("@/hooks/use-assistant-viewport", () => ({ useAssistantViewport: () => useRef(null) }));
vi.mock("@/hooks/use-scroll-fades", () => ({ useScrollFades: () => {} }));
vi.mock("@/hooks/use-theme", () => ({ useApplyTheme: () => {}, useResolvedTheme: () => "dark" }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("./cma-agent-panel", () => ({ CmaAgentPanel: ({ agentId, responseId }: { agentId: string; responseId?: string }) =>
  <button data-agent-id={agentId} data-response-id={responseId}>View execution</button>,
}));
vi.mock("./assistant-account-panel", () => ({ AssistantAccountPanel: () => null }));
vi.mock("./offline-banner", () => ({ OfflineBanner: () => null }));
vi.mock("./assistant-search", () => ({ AssistantSearchButton: () => null, AssistantSearchDialog: () => null }));
vi.mock("./notifications-bell", () => ({ NotificationsBell: () => null }));
vi.mock("@tanstack/react-router", () => ({
  useRouter: () => ({ subscribe: () => () => {} }),
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));

beforeEach(() => {
  useAssistantLayoutStore.setState({ sidebarHidden: false });
  useAssistantWireLogStore.getState().setFeatureEnabled(false);
});

afterEach(() => useAuthStore.setState(initialAuth, true));

function setOwner() {
  useAuthStore.getState().setUser({
    id: "owner",
    email: "owner@example.com",
    display_name: "Owner",
    avatar_url: null,
    is_active: true,
    is_admin: false,
    mfa_enabled: true,
    email_verified: true,
    created_at: "2026-01-01T00:00:00Z",
  });
}

it.each([true, false])(
  "docks an unconfirmed CMA submission outside the transcript (desktop: %s)",
  (desktop) => {
    viewport.desktop = desktop;
    setOwner();
    const view = render(
      <TooltipProvider>
        <AssistantShell title="Chat" sidebar={null} cmaOperations={[receipt]}>
          <section aria-label="Transcript">Assistant reply</section>
        </AssistantShell>
      </TooltipProvider>,
    );
    const card = screen.getByRole("region", {
      name: "CMA submission: CMA setup",
    });
    expect(card).toBeVisible();
    expect(within(card).getByRole("status")).toHaveTextContent(
      "Submission unconfirmed",
    );
    expect(
      within(screen.getByRole("region", { name: "Transcript" })).queryByRole(
        "region",
        { name: "CMA submission: CMA setup" },
      ),
    ).not.toBeInTheDocument();
    expect(card.closest('[aria-label="Preview"]') !== null).toBe(desktop);
    view.rerender(
      <TooltipProvider>
        <AssistantShell title="Another chat" sidebar={null}>
          <section aria-label="Transcript">Another reply</section>
        </AssistantShell>
      </TooltipProvider>,
    );
    expect(
      screen.queryByRole("region", { name: "CMA submission: CMA setup" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("complementary", { name: "Preview" }),
    ).not.toBeInTheDocument();
  },
);

it.each([
  [cmaSession.connection.id, null, false],
  [cmaSession.connection.id, "completed", false],
  [cmaSession.connection.id, "submission_uncertain", true],
  ["another-connection", null, true],
  [undefined, null, true],
] as const)(
  "consolidates only a verified matching live session (connection: %s, reason: %s)",
  (serviceId, reason, showReceipt) => {
    viewport.desktop = true;
    setOwner();
    render(
      <TooltipProvider>
        <CmaSessionContext
          value={[
            {
              session: cmaSession,
              state: initialCmaSession(cmaSession),
              reconnect: () => undefined,
            },
          ]}
        >
          <AssistantShell
            title="Chat"
            sidebar={null}
            cmaOperations={[
              {
                ...receipt,
                service_id: serviceId,
                agent_id: cmaSession.agentId,
                reason,
                state: "waiting",
              },
            ]}
          >
            <section aria-label="Transcript">Assistant reply</section>
          </AssistantShell>
        </CmaSessionContext>
      </TooltipProvider>,
    );
    const preview = screen.getByRole("complementary", { name: "Preview" });
    expect(
      within(preview).getByRole("region", {
        name: `CMA session: ${cmaSession.title}`,
      }),
    ).toBeVisible();
    expect(
      within(preview).queryByRole("region", {
        name: "CMA submission: CMA setup",
      }) !== null,
    ).toBe(showReceipt);
  },
);

it("keeps execution access and elapsed request notices when a live card replaces its receipt", () => {
  viewport.desktop = true;
  setOwner();
  const operation: CmaOperation = {
    ...receipt,
    kind: "cma_response",
    agent_id: cmaSession.agentId,
    agent_ref_id: "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
    response_id: "rsp-linked",
    operation_id: "rsp-linked",
    reason: null,
    status: "in_progress",
    state: "waiting",
    created_at: new Date(Date.now() - 11 * 60_000).toISOString(),
  };
  render(
    <TooltipProvider>
      <CmaSessionContext value={[{
        session: cmaSession,
        state: initialCmaSession(cmaSession),
        reconnect: () => undefined,
      }]}>
        <AssistantShell title="Chat" sidebar={null} cmaOperations={[operation]}>
          <section aria-label="Transcript">Assistant reply</section>
        </AssistantShell>
      </CmaSessionContext>
    </TooltipProvider>,
  );
  const card = screen.getByRole("region", { name: `CMA session: ${cmaSession.title}` });
  expect(within(card).getByRole("button", { name: "View execution" })).toBeVisible();
  expect(within(card).getByText(/CMA still reports it as running/)).not.toBeVisible();
  fireEvent.click(within(card).getByRole("button", { name: "Submission details" }));
  expect(within(card).getByText(/CMA still reports it as running/)).toBeVisible();
  expect(within(card).getByRole("button", { name: "Pin CMA session" })).toBeVisible();
  expect(within(card).getByText("Recent actions")).toBeVisible();
  expect(screen.queryByRole("region", { name: "CMA submission: CMA task" })).not.toBeInTheDocument();
});

function renderCmaOperations(operations: readonly CmaOperation[]) {
  setOwner();
  return render(
    <TooltipProvider>
      <CmaSessionContext value={[{
        session: cmaSession,
        state: { ...initialCmaSession(cmaSession), status: "ready" },
        reconnect: () => undefined,
      }]}>
        <AssistantShell title="Chat" sidebar={null} cmaOperations={operations}>
          <section aria-label="Transcript">Assistant reply</section>
        </AssistantShell>
      </CmaSessionContext>
    </TooltipProvider>,
  );
}

const linkedRequest: CmaOperation = {
  ...receipt,
  kind: "cma_response",
  agent_id: cmaSession.agentId,
  agent_ref_id: "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
  response_id: "rsp-linked",
  operation_id: "rsp-linked",
  reason: null,
  state: "waiting",
  status: "queued",
};

it.each([
  [{ status: "queued" }, "Queued"],
  [{ status: "waiting_approval" }, "Needs approval"],
  [{ status: "connection_required" }, "Needs connection"],
  [{ status: "paused" }, "Paused"],
  [{ state: "submitting", status: null }, "Submitting"],
  [{ status: null }, "Awaiting status"],
  [{ state: "cancelling" }, "Stopping tracking"],
  [{ state: "delivered", status: "completed", reason: "completed" }, "Completed"],
] as const)("retains request status %s alongside the live agent status", (patch, label) => {
  viewport.desktop = true;
  renderCmaOperations([{ ...linkedRequest, ...patch }]);
  const details = screen.getByRole("button", { name: "Submission details" });
  expect(details).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("region", { name: "CMA requests" })).not.toBeInTheDocument();
  expect(screen.getByText("rsp-linked")).not.toBeVisible();
  fireEvent.click(details);
  const requests = screen.getByRole("region", { name: "CMA requests" });
  expect(within(requests).getByRole("status")).toHaveTextContent(label);
  expect(requests.querySelector(".animate-spin")).toBeNull();
  expect(within(requests).getByText("rsp-linked")).toBeVisible();
  expect(within(requests).getByText("request")).toBeVisible();
  expect(within(requests).queryByRole("button", { name: "Submission details" })).not.toBeInTheDocument();
  fireEvent.click(details);
  expect(requests).not.toBeVisible();
});

it("offers one execution panel for the newest active request and bounds older request rows", () => {
  viewport.desktop = true;
  const operations = Array.from({ length: 7 }, (_, index) => ({
    ...linkedRequest,
    id: `request-${index}`,
    response_id: `rsp-${index}`,
    operation_id: `rsp-${index}`,
    created_at: new Date(Date.UTC(2026, 10, 10, index)).toISOString(),
    state: index === 3 || index === 4 ? "waiting" : "delivered",
    status: index === 3 || index === 4 ? "queued" : "completed",
    reason: index === 3 || index === 4 ? null : "completed",
  }));
  renderCmaOperations(operations);
  expect(screen.getAllByRole("button", { name: "Submission details" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Submission details" }));
  const requests = screen.getByRole("region", { name: "CMA requests" });
  const controls = screen.getByRole("group", { name: "CMA session controls" });
  const execution = within(controls).getAllByRole("button", { name: "View execution" });
  expect(execution).toHaveLength(1);
  expect(execution[0]).toHaveAttribute("data-agent-id", linkedRequest.agent_ref_id);
  expect(execution[0]).toHaveAttribute("data-response-id", "rsp-4");
  expect(requests.querySelectorAll("[data-cma-request]")).toHaveLength(3);
  expect(requests.querySelector("[data-cma-request]")).toHaveAttribute("data-cma-request", "request-4");
  fireEvent.click(within(requests).getByRole("button", { name: "Show older requests (4)" }));
  expect(requests.querySelectorAll("[data-cma-request]")).toHaveLength(6);
  expect(within(controls).getAllByRole("button", { name: "View execution" })).toHaveLength(1);
  expect(within(requests).queryByRole("button", { name: "View execution" })).not.toBeInTheDocument();
  expect(screen.getAllByRole("button", { name: "Submission details" })).toHaveLength(1);
});

it.each([true, false])("keeps live work ahead of a bounded submission history (desktop: %s)", (desktop) => {
  viewport.desktop = desktop;
  renderCmaOperations(Array.from({ length: 20 }, (_, index) => ({ ...receipt, id: `older-${index}` })));
  const stack = screen.getByRole("complementary", { name: "CMA sessions" });
  expect(stack.firstElementChild).toHaveAttribute("aria-label", `CMA session: ${cmaSession.title}`);
  expect(stack.querySelectorAll("[data-cma-operation]")).toHaveLength(3);
  fireEvent.click(within(stack).getByRole("button", { name: "Show older CMA submissions (17)" }));
  expect(stack.querySelectorAll("[data-cma-operation]")).toHaveLength(6);
});

it.each([true, false])("has one title and Rename for legacy, workspace, loading and resolved threads (desktop: %s)", (desktop) => {
  viewport.desktop = desktop;
  const rename = vi.fn().mockResolvedValue(undefined);
  const view = render(<AssistantShell title="Legacy chat" sidebar={null} onRenameTitle={rename}><p>Transcript</p></AssistantShell>);
  expect(screen.getAllByText("Legacy chat")).toHaveLength(1);
  expect(screen.getAllByRole("button", { name: "Rename chat" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Rename chat" }));
  expect(screen.getByRole("textbox", { name: "Chat title" })).toHaveValue("Legacy chat");
  fireEvent.click(screen.getByRole("button", { name: "Cancel rename" }));
  view.rerender(<AssistantShell title="Automations" sidebar={null}><p>Workspace</p></AssistantShell>);
  expect(screen.getAllByRole("heading", { name: "Automations" })).toHaveLength(1);
  expect(screen.queryByRole("button", { name: "Rename chat" })).not.toBeInTheDocument();
  view.rerender(<AssistantShell title="Loading thread" sidebar={null} onRenameTitle={rename}><p>Loading agent</p></AssistantShell>);
  expect(screen.getAllByText("Loading thread")).toHaveLength(1);
  expect(screen.getAllByRole("button", { name: "Rename chat" })).toHaveLength(1);
  view.rerender(<AssistantShell title="Resolved thread" contentTitle={desktop} sidebar={null} onRenameTitle={rename}>
    <ThreadHeader name="NyxBot" kind="nyxbot" destroyed={false} title={desktop ? "Resolved thread" : undefined} onRenameTitle={rename} />
  </AssistantShell>);
  expect(screen.getAllByText("Resolved thread")).toHaveLength(1);
  expect(screen.getAllByRole("button", { name: "Rename chat" })).toHaveLength(1);
});

it.each([true, false])("mounts wire capture and mock scenarios once at every width (desktop: %s)", (desktop) => {
  viewport.desktop = desktop;
  render(<QueryClientProvider client={new QueryClient()}><TooltipProvider>
    <AssistantShell title="Chat" sidebar={null} headerActions={<><AssistantWireLogAction activeConversationId={null} /><MockScenariosAction /></>}><p>Transcript</p></AssistantShell>
  </TooltipProvider></QueryClientProvider>);
  expect(useAssistantWireLogStore.getState().featureEnabled).toBe(true);
  expect(screen.getAllByRole("button", { name: "Mock scenarios" })).toHaveLength(1);
  expect(screen.getAllByRole("button", { name: /Wire log/i })).toHaveLength(1);
});
