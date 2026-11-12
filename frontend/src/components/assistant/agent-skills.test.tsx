import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { AgentSkills } from "./agent-skills";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { User } from "@/types/api";

function signIn(features: string[] = []) {
  useAuthStore.setState({
    user: { id: "owner", capabilities: { enabled_features: features } } as unknown as User,
  });
}
const mocks = vi.hoisted(() => ({
  current: vi.fn(),
  save: vi.fn(),
  toggle: vi.fn(),
  search: vi.fn(),
  versions: vi.fn(),
  preview: vi.fn(),
  attachedPreview: vi.fn(),
  history: vi.fn(),
  openPanel: vi.fn(),
}));
vi.mock("@/hooks/use-account-panel", () => ({
  useAccountPanel: () => ({ open: mocks.openPanel }),
}));
vi.mock("@/hooks/use-agent-skills", () => ({
  useAgentSkills: mocks.current,
  useToggleSystemSkill: () => ({
    mutate: mocks.toggle,
    isPending: false,
    error: null,
  }),
  useSetSkillPreload: () => ({ mutate: vi.fn(), isPending: false, error: null }),
  useSetAgentSkills: () => ({ mutate: mocks.save, isPending: false }),
  useSkillSearch: mocks.search,
  useSkillVersions: mocks.versions,
  useSkillPreview: mocks.preview,
  useAttachedSkillPreview: mocks.attachedPreview,
  useAgentSkillHistory: mocks.history,
  useImproveAgentSkill: () => ({ mutate: vi.fn(), isPending: false, error: null }),
}));
const pin = {
  source: "ornn",
  skill_id: "skill-id",
  name: "Research",
  version: "1.0",
  sha256: "a".repeat(64),
  dependencies: [],
};
beforeEach(() => {
  vi.clearAllMocks();
  signIn();
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [pin],
      metadata: {
        "skill-id": { description: "Research guide", size_bytes: 1000 },
      },
    },
  });
  mocks.attachedPreview.mockReturnValue({ data: undefined, isFetching: false, error: null });
  mocks.history.mockReturnValue({ data: { items: [] } });
  mocks.search.mockReturnValue({ data: undefined });
  mocks.versions.mockReturnValue({
    data: {
      items: [
        { version: "2.0", sha256: "b".repeat(64), deprecated: false },
        { version: "1.0", sha256: pin.sha256, deprecated: false },
      ],
    },
  });
  mocks.preview.mockReturnValue({ data: undefined });
});
it("shows pinned metadata, revision and an explicit update action without re-pinning", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  expect(screen.getByText("Research guide", { selector: "[data-grouped-footer] p" })).toBeInTheDocument();
  expect(screen.getByText("1,000 bytes")).toBeInTheDocument();
  expect(screen.getByText(/Revision 4/)).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Newer version available: 2.0" }),
  ).toBeInTheDocument();
  expect(mocks.save).not.toHaveBeenCalled();
});
it("shows built-in choices in history without labelling other skill events as inherited", () => {
  mocks.history.mockReturnValue({
    data: {
      items: [
        ["assistant_system_skill_toggled", "built-in-inherit", null],
        ["assistant_system_skill_toggled", "built-in-on", true],
        ["assistant_system_skill_toggled", "built-in-off", false],
        ["assistant_skill_preload_updated", "preloaded-skill", null],
        ["assistant_learning_skill_pinned", "published-skill", null],
      ].map(([event_type, skill_id, enabled], index) => ({
        id: String(index),
        event_type,
        skill_id,
        enabled,
        created_at: "2026-11-10T00:00:00Z",
        revision: null,
        skills: [],
      })),
    },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  const items = within(
    screen.getByRole("list", { name: "Skill history" }),
  ).getAllByRole("listitem");
  expect(items[0]).toHaveTextContent("built-in-inherit · inherited");
  expect(items[1]).toHaveTextContent("built-in-on · enabled");
  expect(items[2]).toHaveTextContent("built-in-off · disabled");
  expect(items[3]).toHaveTextContent("Skill preload settings changed");
  expect(items[3]).not.toHaveTextContent("inherited");
  expect(items[4]).toHaveTextContent("Published skill attached");
  expect(items[4]).not.toHaveTextContent("inherited");
});
it("removes immediately with the observed revision", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  fireEvent.click(screen.getByRole("button", { name: "Remove Research" }));
  expect(mocks.save).toHaveBeenCalledWith(
    { expected_revision: 4, skills: [] },
    expect.anything(),
  );
});
it("hides the enable toggle while assistant:skill-pin-disable is off", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  expect(screen.getByText("Research guide", { selector: "[data-grouped-footer] p" })).toBeInTheDocument();
  expect(screen.queryByRole("switch", { name: "Use Research" })).not.toBeInTheDocument();
  expect(mocks.save).not.toHaveBeenCalled();
});
it("still marks a disabled pin when the toggle is hidden", () => {
  mocks.current.mockReturnValue({
    data: { revision: 4, skills: [{ ...pin, disabled: true }], metadata: {} },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  expect(screen.queryByRole("switch", { name: "Use Research" })).not.toBeInTheDocument();
  expect(screen.getByText("Disabled")).toBeInTheDocument();
});
it("toggles a pin and sends disabled true in the PUT body", () => {
  signIn([FEATURE_FLAG.SKILL_PIN_DISABLE]);
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  fireEvent.click(screen.getByRole("switch", { name: "Use Research" }));
  expect(mocks.save).toHaveBeenCalledWith({ expected_revision: 4, skills: [{ ...pin, disabled: true }] }, expect.anything());
});
it("searches visible Ornn skills on submit", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.change(
    screen.getByRole("textbox", { name: "Search Ornn skills" }),
    { target: { value: "private research" } },
  );
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
  expect(mocks.search).toHaveBeenLastCalledWith("private research", 1);
});
it("requires version preview and an explicit re-pin click", async () => {
  const user = userEvent.setup();
  const view = render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  await user.click(
    screen.getByRole("button", { name: "Newer version available: 2.0" }),
  );
  expect(screen.getByRole("button", { name: "Re-pin skill" })).toBeDisabled();
  await user.click(screen.getByRole("combobox", { name: "Skill version" }));
  await user.click(screen.getByRole("option", { name: "2.0" }));
  expect(mocks.preview).toHaveBeenLastCalledWith("skill-id", "2.0");
  const next = { ...pin, version: "2.0", sha256: "b".repeat(64) };
  mocks.preview.mockReturnValue({
    data: { reference: next, description: "New guidance", size_bytes: 2400 },
  });
  view.rerender(<AgentSkills agentId="agent" />);
  expect(mocks.save).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Re-pin skill" }));
  await waitFor(() =>
    expect(mocks.save).toHaveBeenCalledWith(
      { expected_revision: 4, skills: [next] },
      expect.anything(),
    ),
  );
});
it("shows the skill's own outline card when the verified preview carries one", async () => {
  mocks.search.mockReturnValue({
    data: { page: 1, total_pages: 1, items: [{ id: "style", name: "style", description: "House style" }] },
  });
  const reference = { ...pin, skill_id: "style", name: "style", version: "2.0" };
  mocks.preview.mockReturnValue({
    data: {
      reference,
      description: "House style",
      size_bytes: 900,
      outline: {
        overview: "One result.",
        steps: [{ id: "step-1", title: "Prepare", text: "I get ready.", asks_you: true }],
        needs: null,
        produces: null,
      },
      author: { kind: "organization", name: "Example Org" },
    },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^style/ }));
  const card = await screen.findByRole("region", { name: "style skill" });
  expect(card).toHaveTextContent("By Example Org");
  expect(card).toHaveTextContent("Prepare. I get ready.");
  expect(card).toHaveTextContent("Asks you");
  // Attaching stays with the version view's own controls.
  expect(screen.queryByRole("button", { name: "Start" })).not.toBeInTheDocument();
});
it("shows no outline card when the server sends no outline", async () => {
  mocks.search.mockReturnValue({
    data: { page: 1, total_pages: 1, items: [{ id: "style", name: "style", description: "House style" }] },
  });
  mocks.preview.mockReturnValue({
    data: { reference: { ...pin, skill_id: "style", name: "style", version: "2.0" }, description: "House style", size_bytes: 900 },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^style/ }));
  expect(await screen.findByText("900 bytes ·", { exact: false })).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "style skill" })).not.toBeInTheDocument();
});
it("shows missing-access errors without erasing attached skills", () => {
  mocks.search.mockReturnValue({
    error: new Error("Skill unavailable through your Ornn access"),
  });
  render(<AgentSkills agentId="agent" />);
  expect(screen.getByText(/Skill unavailable/)).toBeInTheDocument();
  expect(screen.getByText("Research guide")).toBeInTheDocument();
});
it("shows the legacy empty state and prevents changes for destroyed agents", () => {
  mocks.current.mockReturnValue({
    data: { revision: 0, skills: [], metadata: {} },
  });
  render(<AgentSkills agentId="agent" readOnly />);
  expect(screen.getByText("No skills attached.")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Search" }),
  ).not.toBeInTheDocument();
});

it("offers paging through older versions before re-pinning", () => {
  mocks.versions.mockReturnValue({
    data: {
      page: 1,
      total_pages: 2,
      items: [{ version: "2.0", sha256: "b".repeat(64), deprecated: false }],
    },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  fireEvent.click(
    screen.getByRole("button", { name: "Newer version available: 2.0" }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Older versions" }));
  expect(mocks.versions).toHaveBeenCalledWith("skill-id", 2);
  expect(screen.getByRole("button", { name: "Re-pin skill" })).toBeDisabled();
  expect(mocks.save).not.toHaveBeenCalled();
});
it("refuses a seventeenth attachment while retaining removal", () => {
  mocks.current.mockReturnValue({
    data: {
      revision: 9,
      skills: Array.from({ length: 16 }, (_, i) => ({
        ...pin,
        skill_id: `s${i}`,
        name: `Skill ${i}`,
      })),
      metadata: {},
    },
  });
  mocks.search.mockReturnValue({
    data: {
      items: [{ id: "seventeenth", name: "Extra skill", description: "Extra" }],
      page: 1,
      total_pages: 1,
    },
  });
  mocks.preview.mockReturnValue({
    data: {
      reference: { ...pin, skill_id: "seventeenth" },
      description: "Extra",
      size_bytes: 100,
    },
  });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /Extra skill/ }));
  expect(screen.getByRole("button", { name: "Attach skill" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Back to Skills" }));
  fireEvent.click(screen.getByRole("button", { name: /^Skill 0 / }));
  expect(screen.getByRole("button", { name: "Remove Skill 0" })).toBeEnabled();
});

it("never tests a pin until its Test button is clicked", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: /^Research/ }));
  expect(mocks.attachedPreview).toHaveBeenLastCalledWith("agent", "");
  fireEvent.click(screen.getByRole("button", { name: "Test Research" }));
  expect(mocks.attachedPreview).toHaveBeenLastCalledWith("agent", "skill-id");
});

it("shows a history request failure on the pushed history page", () => {
  mocks.history.mockReturnValue({ error: new Error("History request failed") });
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  expect(screen.getByRole("heading", { name: "Skill history" })).toHaveFocus();
  expect(screen.getByRole("alert")).toHaveTextContent("History request failed");
  expect(screen.queryByText("No skill activity yet.")).toBeNull();
});

it("shows built-ins, toggles through the owner endpoint and tests enabled skills", () => {
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [],
      metadata: {},
      system_skills: [
        {
          skill_id: "builtin",
          name: "NyxBot guide",
          version: "1.0",
          description: "Built-in guide",
          enabled: true,
          effective: true,
          override: null,
          source: "nyxbot_default",
          shadowed_by_pin: false,
        },
      ],
    },
  });
  render(<AgentSkills agentId="agent" />);
  expect(screen.getByText("Built-in guide")).toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("switch", { name: "Use built-in NyxBot guide" }),
  );
  expect(mocks.toggle).toHaveBeenCalledWith({
    skillId: "builtin",
    enabled: false,
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Test built-in NyxBot guide" }),
  );
  expect(mocks.attachedPreview).toHaveBeenLastCalledWith("agent", "builtin");
  expect(mocks.save).not.toHaveBeenCalled();
});
it("makes built-ins read-only for viewers and explains pin precedence", () => {
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [],
      metadata: {},
      system_skills: [
        {
          skill_id: "builtin",
          name: "NyxBot guide",
          version: "1.0",
          description: "Built-in guide",
          enabled: true,
          effective: false,
          override: null,
          source: "pin",
          shadowed_by_pin: true,
        },
      ],
    },
  });
  render(<AgentSkills agentId="agent" readOnly />);
  expect(
    screen.getByRole("switch", { name: "Use built-in NyxBot guide" }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Your attached pin takes precedence/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Test built-in NyxBot guide" }),
  ).not.toBeInTheDocument();
});
it("does not offer Test for an opted-out built-in", () => {
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [],
      metadata: {},
      system_skills: [
        {
          skill_id: "builtin",
          name: "NyxBot guide",
          version: "1.0",
          description: "Built-in guide",
          enabled: false,
          effective: false,
          override: false,
          source: "agent",
          shadowed_by_pin: false,
        },
      ],
    },
  });
  render(<AgentSkills agentId="agent" />);
  expect(
    screen.getByRole("switch", { name: "Use built-in NyxBot guide" }),
  ).not.toBeChecked();
  expect(
    screen.queryByRole("button", { name: "Test built-in NyxBot guide" }),
  ).not.toBeInTheDocument();
});

it.each([
  {
    mode: "default",
    own: null,
    source: "nyxbot_default",
    label: "Inherited from NyxBot",
    locked: false,
  },
  {
    mode: "default",
    own: false,
    source: "agent",
    label: "Your choice for this agent",
    locked: false,
  },
  {
    mode: "override",
    own: true,
    source: "nyxbot_override",
    label: "Set by NyxBot for all specialists",
    locked: true,
  },
])(
  "shows specialist inheritance: $label",
  ({ mode, own, source, label, locked }) => {
    mocks.current.mockReturnValue({
      data: {
        revision: 4,
        skills: [],
        metadata: {},
        system_skills_for_specialists: mode,
        system_skills: [
          {
            skill_id: "builtin",
            name: "Guide",
            version: "1.0",
            description: "Guide content",
            enabled: false,
            effective: false,
            override: own,
            source,
            shadowed_by_pin: false,
          },
        ],
      },
    });
    render(<AgentSkills agentId="agent" kind="specialist" />);
    expect(screen.getByText(label)).toBeInTheDocument();
    const toggle = screen.getByRole("switch", { name: "Use built-in Guide" });
    if (locked) expect(toggle).toBeDisabled();
    else expect(toggle).toBeEnabled();
    const reset = screen.queryByRole("button", { name: "Use NyxBot’s choice" });
    if (own !== null && !locked) {
      fireEvent.click(reset!);
      expect(mocks.toggle).toHaveBeenCalledWith({
        skillId: "builtin",
        enabled: null,
      });
    } else expect(reset).not.toBeInTheDocument();
  },
);

it("shows NyxBot’s specialist mode and settings link without locking NyxBot", () => {
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [],
      metadata: {},
      system_skills_for_specialists: "override",
      system_skills: [
        {
          skill_id: "builtin",
          name: "Guide",
          version: "1.0",
          description: "Guide content",
          enabled: true,
          effective: true,
          override: null,
          source: "platform",
          shadowed_by_pin: false,
        },
      ],
    },
  });
  render(<AgentSkills agentId="agent" kind="nyxbot" />);
  expect(
    screen.getByText("NyxBot’s choices apply to all your specialists."),
  ).toBeInTheDocument();
  const settings = screen.getByRole("button", { name: "NyxBot settings" });
  fireEvent.click(settings);
  expect(mocks.openPanel).toHaveBeenCalledWith("nyxbot", {}, settings);
  expect(
    screen.getByRole("switch", { name: "Use built-in Guide" }),
  ).toBeEnabled();
});
