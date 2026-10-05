import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentLearningReview } from "./agent-learning-review";

const mocks = vi.hoisted(() => ({
  feature: vi.fn(),
  review: vi.fn(),
  approve: vi.fn(),
  configure: vi.fn(),
  edit: vi.fn(),
  reject: vi.fn(),
  run: vi.fn(),
}));

vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: mocks.feature }));
vi.mock("@/hooks/use-agent-learning-review", () => ({
  useAgentLearningReview: mocks.review,
}));

const proposal = {
  id: "proposal",
  agent_id: "agent",
  status: "pending",
  revision: 2,
  config_revision: 1,
  agent_skills_revision: 3,
  evidence_count: 2,
  body_bytes: 42,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  draft: {
    schema_version: 1,
    kind: "new" as const,
    name: "Untrusted skill",
    description: "A reusable procedure",
    skill_md: "# Do not execute this as markup",
    rationale: "Reusable",
    safety_notes: "Review before use",
    files: [],
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.feature.mockReturnValue(false);
  mocks.review.mockReturnValue({
    data: { proposals: [proposal] },
    status: { data: { enabled: true } },
    configure: { mutate: mocks.configure, isPending: false },
    approve: { mutateAsync: mocks.approve, isPending: false },
    edit: { mutate: mocks.edit, isPending: false },
    reject: { mutate: mocks.reject, isPending: false },
    run: { mutate: mocks.run, isPending: false },
  });
  mocks.approve.mockResolvedValue({
    status: "confirmation_required",
    acknowledgement: { acknowledgement_id: "card" },
  });
});

describe("AgentLearningReview", () => {
  it("hides the section and does not fetch while the flag is off", () => {
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.queryByRole("region", { name: "Learning" })).not.toBeInTheDocument();
    expect(mocks.review).toHaveBeenCalledWith("agent", false);
  });

  it("uses the review primitives, keeps drafts as plain text, and binds the two-step card", async () => {
    mocks.feature.mockReturnValue(true);
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByRole("region", { name: "Learning" })).toBeInTheDocument();
    expect(screen.getByRole("switch")).toBeChecked();
    expect(screen.getByRole("spinbutton", { name: "Learning threshold" })).toHaveValue(15);
    fireEvent.click(screen.getByRole("button", { name: "Review draft" }));
    expect(screen.getByRole("textbox", { name: "Skill draft" })).toHaveValue(
      "# Do not execute this as markup",
    );
    expect(screen.getByText("Review before use")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Publish & request confirmation" }));
    await waitFor(() => expect(mocks.approve).toHaveBeenCalledWith({ proposalId: "proposal", acknowledgementId: undefined }));
    expect(await screen.findByText(/Confirming this exact card/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm publish & attach" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Confirm publish & attach" }));
    await waitFor(() => expect(mocks.approve).toHaveBeenLastCalledWith({ proposalId: "proposal", acknowledgementId: "card" }));
  });
});
