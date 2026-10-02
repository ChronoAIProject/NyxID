import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { AgentSkills } from "./agent-skills";
const mocks = vi.hoisted(() => ({
  current: vi.fn(),
  save: vi.fn(),
  search: vi.fn(),
  versions: vi.fn(),
  preview: vi.fn(),
}));
vi.mock("@/hooks/use-agent-skills", () => ({
  useAgentSkills: mocks.current,
  useSetAgentSkills: () => ({ mutate: mocks.save, isPending: false }),
  useSkillSearch: mocks.search,
  useSkillVersions: mocks.versions,
  useSkillPreview: mocks.preview,
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
  mocks.current.mockReturnValue({
    data: {
      revision: 4,
      skills: [pin],
      metadata: {
        "skill-id": { description: "Research guide", size_bytes: 1000 },
      },
    },
  });
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
  expect(screen.getByText("Research guide")).toBeInTheDocument();
  expect(screen.getByText("1,000 bytes")).toBeInTheDocument();
  expect(screen.getByText(/Revision 4/)).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Newer version available: 2.0" }),
  ).toBeInTheDocument();
  expect(mocks.save).not.toHaveBeenCalled();
});
it("removes immediately with the observed revision", () => {
  render(<AgentSkills agentId="agent" />);
  fireEvent.click(screen.getByRole("button", { name: "Remove Research" }));
  expect(mocks.save).toHaveBeenCalledWith(
    { expected_revision: 4, skills: [] },
    expect.anything(),
  );
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
  fireEvent.click(screen.getByRole("button", { name: "Extra skill" }));
  expect(screen.getByRole("button", { name: "Attach skill" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Remove Skill 0" })).toBeEnabled();
});
