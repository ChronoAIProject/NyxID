import { confirmChanges } from "@/test-helpers/confirm-changes";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentSkills } from "@/components/assistant/agent-skills";
import { AdminAssistantSystemSkillsPage } from "@/pages/admin-assistant-system-skills";
import { NyxBotSettingsContent } from "@/components/assistant/nyxbot-settings-content";
import { getMockUser, getMockResponse } from "@/lib/mock-data";
import { installAssistantHttpFixtures } from "@/lib/assistant/assistant-http-fixtures";
import { NYXAGENT_FIXTURE_NYXBOT_ID, NYXAGENT_FIXTURE_RESEARCHER_ID } from "@/lib/assistant/nyxagent-http-fixtures";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
vi.mock("@/hooks/use-account-panel", () => ({ useAccountPanel: () => ({ open: vi.fn() }) }));
vi.mock("@/components/assistant/automation-preferences", () => ({ AutomationPreferences: () => null }));
vi.mock("@/components/assistant/platform-personal-bindings", () => ({ PlatformPersonalBindings: () => null }));
vi.mock("@/components/assistant/channel-adapter-bindings", () => ({ ChannelAdapterBindings: () => null }));
vi.mock("@/components/assistant/nyxbot-channels", () => ({ ChannelBotsManager: () => null }));
let client: QueryClient;
let network: ReturnType<typeof vi.fn>;
let previousUrl: string;
beforeEach(() => {
  previousUrl = window.location.href;
  window.history.replaceState(null, "", "/assistant?mock=1&nyxbot=1&systemSkills=default");
  sessionStorage.clear();
  globalThis.__nyxidAssistantHttpMock = undefined;
  globalThis.__nyxidAssistantHttpFixtureWorld = undefined;
  useAuthStore.setState({ user: getMockUser(), isAuthenticated: true });
  network = vi.fn();
  vi.stubGlobal("fetch", network);
  installAssistantHttpFixtures();
  client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
});
afterEach(() => {
  cleanup(); client.clear();
  expect(network).not.toHaveBeenCalled();
  window.history.replaceState(null, "", previousUrl);
  globalThis.__nyxidAssistantHttpMock = undefined;
  globalThis.__nyxidAssistantHttpFixtureWorld = undefined;
  vi.unstubAllGlobals();
  sessionStorage.clear();
});
function show(component: React.ReactNode) {
  return render(<QueryClientProvider client={client}>{component}</QueryClientProvider>);
}
it("renders and saves the admin page using the development transport", async () => {
  show(<AdminAssistantSystemSkillsPage />);
  await screen.findByText("Coordinate tasks and work with your specialist team.");
  fireEvent.click(screen.getByRole("button", { name: "Move nyxbot-ornn-skills up" }));
  const save = screen.getByRole("button", { name: "Save built-in skills" });
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);
  await waitFor(() => expect(getMockResponse("/admin/assistant/system-skills")).toMatchObject({ revision: 2 }));
});
it("searches, selects a version and previews through the admin picker with the engine off", async () => {
  const admin = getMockUser();
  useAuthStore.setState({ user: {
    ...admin,
    capabilities: { ...admin.capabilities, enabled_features: [] },
  } });
  const user = userEvent.setup();
  show(<AdminAssistantSystemSkillsPage />);
  await screen.findByText("Coordinate tasks and work with your specialist team.");
  await user.click(screen.getByRole("button", { name: "Remove nyxbot" }));
  await user.click(screen.getByRole("button", { name: "Search" }));
  await user.click(await screen.findByRole("button", { name: "nyxbot" }));
  await user.click(screen.getByRole("combobox", { name: "Skill version" }));
  await user.click(await screen.findByRole("option", { name: "0.1" }));
  await user.click(await screen.findByRole("button", { name: "Add skill" }));
  await user.click(screen.getByRole("button", { name: "Save built-in skills" }));
  await waitFor(() => expect(getMockResponse("/admin/assistant/system-skills")).toMatchObject({
    revision: 2,
    skills: [
      { name: "nyxbot-ornn-skills" }, { name: "nyxbot-cma" }, { name: "nyxbot" },
    ],
  }));
});
it("renders NyxBot Skills and its placeholder built-in preview", async () => {
  show(<AgentSkills agentId={NYXAGENT_FIXTURE_NYXBOT_ID} kind="nyxbot" />);
  await screen.findByRole("heading", { name: "Built-in skills" });
  expect(screen.getByRole("switch", { name: "Use built-in nyxbot" })).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Test built-in nyxbot" }));
  await screen.findByText(/This placeholder SKILL.md is served by the local development fixture/);
});
it("renders specialist defaults, its own choice and pin shadowing", async () => {
  show(<AgentSkills agentId={NYXAGENT_FIXTURE_RESEARCHER_ID} />);
  expect((await screen.findAllByText("Inherited from NyxBot")).length).toBeGreaterThan(0);
  expect(screen.getByText("Your choice for this agent")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use NyxBot’s choice" })).toBeEnabled();
  expect(screen.getByText(/Your attached pin takes precedence/)).toBeInTheDocument();
});
it("renders specialist override mode with locked built-in controls", async () => {
  await assistantJson("/assistant/nyxagent/settings", { method: "PUT", body: { system_skills_for_specialists: "override" } });
  show(<AgentSkills agentId={NYXAGENT_FIXTURE_RESEARCHER_ID} />);
  await screen.findByRole("heading", { name: "Built-in skills" });
  const switches = screen.getAllByRole("switch", { name: /^Use built-in / });
  expect(switches).toHaveLength(3);
  switches.forEach((control) => expect(control).toBeDisabled());
});
it("renders and saves the NyxBot specialist mode control with the real settings hook", async () => {
  const user = userEvent.setup();
  show(<NyxBotSettingsContent />);
  await screen.findByRole("combobox", { name: "Built-in skills for specialists" });
  await user.click(screen.getByRole("combobox", { name: "Built-in skills for specialists" }));
  await user.click(screen.getByRole("option", { name: "Apply NyxBot’s choices to all specialists" }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await confirmChanges();
  await waitFor(async () => expect(await assistantJson("/assistant/nyxagent/settings")).toMatchObject({ system_skills_for_specialists: "override" }));
});
