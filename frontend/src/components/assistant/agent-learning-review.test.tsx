import type { ComponentProps } from "react";
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
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));
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
    reject: { mutateAsync: mocks.reject, isPending: false },
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

  it("keeps authored proposals visible without the invalid approval button", () => {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals: [{ ...proposal, source: "authored" }] },
      status: { data: { enabled: true } },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      edit: { mutate: mocks.edit, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText(/Confirm this authored skill from its conversation card/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Confirm publish & attach" })).not.toBeInTheDocument();
  });

  it("keeps a verified stale-evidence publication visible without another check", () => {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals: [{ ...proposal, status: "published_unpinned", failure_code: "evidence_unavailable", evidence_available: false }] },
      status: { data: { enabled: true } },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      edit: { mutate: mocks.edit, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText(/it will\s+not attach it to the agent/)).toBeVisible();
    expect(screen.getByText(/verified this private version in Ornn but did not attach it/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Check publication" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
    expect(mocks.approve).not.toHaveBeenCalled();
  });

  it("checks an unverified stale-evidence publication without offering to attach it", async () => {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals: [{ ...proposal, status: "publication_failed", failure_code: "publish_uncertain", evidence_available: false }] },
      status: { data: { enabled: true } },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      edit: { mutate: mocks.edit, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Check publication" }));
    await waitFor(() => expect(mocks.approve).toHaveBeenCalledWith({ proposalId: "proposal", acknowledgementId: undefined }));
    expect(await screen.findByRole("button", { name: "Confirm check" })).toBeVisible();
    expect(screen.queryByText(/Confirming this exact card/)).not.toBeInTheDocument();
  });

  function withProposals(proposals: unknown[]) {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals },
      status: { data: { enabled: true } },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      edit: { mutate: mocks.edit, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
    });
  }

  it("links an authored proposal to its conversation card and shows its state", () => {
    withProposals([{ ...proposal, source: "authored", status: "publication_failed", failure_code: "publish_uncertain", card_conversation_id: "conversation-1" }]);
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByRole("link", { name: "Open the confirmation card" })).toHaveAttribute("href", "/assistant?c=conversation-1");
    expect(screen.getByText(/cannot yet confirm whether Ornn published this version/)).toBeVisible();
    expect(screen.getByText("publication_failed")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Publish & request confirmation" })).not.toBeInTheDocument();
  });

  it("shows a refused learned confirmation instead of failing silently", async () => {
    withProposals([{ ...proposal, failure_code: "version_conflict" }]);
    mocks.approve.mockRejectedValue(new Error("Learning approval card is missing, expired, used or stale"));
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText(/This version exists with different content/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Publish & request confirmation" }));
    expect(await screen.findByText("Learning approval card is missing, expired, used or stale")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Confirm publish & attach" })).not.toBeInTheDocument();
  });

  it("explains a verified learned version that was not attached", () => {
    withProposals([{ ...proposal, status: "published_unpinned", failure_code: "evidence_unavailable", evidence_available: false }]);
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText(/verified this private version in Ornn but did not attach it/)).toBeVisible();
  });

  it("shows the server's reason when a reject is refused", async () => {
    withProposals([{ ...proposal, status: "publication_failed", failure_code: "publish_uncertain" }]);
    mocks.reject.mockRejectedValue(new Error("Publication must be reconciled before the draft can change"));
    render(<AgentLearningReview agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    await waitFor(() => expect(mocks.reject).toHaveBeenCalledWith({ proposalId: "proposal", revision: 2 }));
    expect(await screen.findByText("Publication must be reconciled before the draft can change")).toBeVisible();
  });

  it("offers Reject only for drafts the server can reject", () => {
    withProposals([
      { ...proposal, id: "settled", status: "published_unpinned", failure_code: "evidence_unavailable", evidence_available: false },
      { ...proposal, id: "running", status: "publishing" },
      { ...proposal, id: "pending", status: "pending" },
    ]);
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getAllByRole("button", { name: "Reject" })).toHaveLength(1);
  });
});
