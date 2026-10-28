import { useRef, type ReactNode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { useAssistantLayoutStore } from "@/stores/assistant-layout-store";
import { AssistantShell } from "./assistant-shell";
import { ThreadHeader } from "./nyxbot-agent-panels";
import { AssistantWireLogAction } from "./assistant-wire-log-panel";
import { MockScenariosAction } from "./mock-scenarios-action";

const viewport = vi.hoisted(() => ({ desktop: true }));
vi.mock("@/hooks/use-media-query", () => ({ useMediaQuery: () => viewport.desktop }));
vi.mock("@/hooks/use-assistant-viewport", () => ({ useAssistantViewport: () => useRef(null) }));
vi.mock("@/hooks/use-scroll-fades", () => ({ useScrollFades: () => {} }));
vi.mock("@/hooks/use-theme", () => ({ useApplyTheme: () => {}, useResolvedTheme: () => "dark" }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("./assistant-account-panel", () => ({ AssistantAccountPanel: () => null }));
vi.mock("./offline-banner", () => ({ OfflineBanner: () => null }));
vi.mock("./assistant-search", () => ({ AssistantSearchButton: () => null, AssistantSearchDialog: () => null }));
vi.mock("@tanstack/react-router", () => ({
  useRouter: () => ({ subscribe: () => () => {} }),
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));

beforeEach(() => {
  useAssistantLayoutStore.setState({ sidebarHidden: false });
  useAssistantWireLogStore.getState().setFeatureEnabled(false);
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
  view.rerender(<AssistantShell title="Resolved thread" contentTitle sidebar={null} onRenameTitle={rename}>
    <ThreadHeader name="NyxBot" kind="nyxbot" destroyed={false} title="Resolved thread" onRenameTitle={rename} />
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
