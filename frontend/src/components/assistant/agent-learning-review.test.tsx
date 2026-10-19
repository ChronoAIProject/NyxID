import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ComponentProps } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentLearningReview } from "./agent-learning-review";
import { ApiError } from "@/lib/api-client";

const mocks = vi.hoisted(() => ({
  feature: vi.fn(),
  json: vi.fn(),
  review: vi.fn(),
  approve: vi.fn(),
  configure: vi.fn(),
  edit: vi.fn(),
  validate: vi.fn(),
  reject: vi.fn(),
  run: vi.fn(),
  consent: vi.fn(),
}));

vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: mocks.json }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (state: unknown) => unknown) => select({ user: { id: "owner" } }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: mocks.feature }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));
vi.mock("@/hooks/use-agent-learning-review", () => ({
  useAgentLearningReview: mocks.review,
}));
// The draft editor reads attached pins only for improve drafts.
vi.mock("@/hooks/use-agent-skills", () => ({
  useAgentSkills: () => ({ data: undefined, error: null }),
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
    validate: { mutateAsync: mocks.validate, isPending: false },
    reject: { mutateAsync: mocks.reject, isPending: false },
    run: { mutate: mocks.run, isPending: false },
    consent: { mutate: mocks.consent, isPending: false },
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

  it("clicks Save draft with a top-level revision and rejects the edited revision", async () => {
    mocks.feature.mockReturnValue(true);
    render(<AgentLearningReview agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Review draft" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Skill draft" }), { target: { value: "# Edited" } });
    fireEvent.click(screen.getByRole("button", { name: "Save draft" }));
    await waitFor(() => expect(mocks.edit).toHaveBeenCalledWith(expect.objectContaining({ proposalId: "proposal", revision: 2 })));
    fireEvent.click(screen.getByRole("button", { name: "Reject" }));
    await waitFor(() => expect(mocks.reject).toHaveBeenCalledWith({ proposalId: "proposal", revision: 2 }));
  });

  it("saves a changed name in the edit payload", async () => {
    mocks.feature.mockReturnValue(true);
    render(<AgentLearningReview agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Review draft" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Skill name" }), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save draft" }));
    // The full draft is sent, including the unchanged SKILL.md.
    await waitFor(() => expect(mocks.edit).toHaveBeenCalledWith({
      proposalId: "proposal",
      revision: 2,
      draft: { ...proposal.draft, name: "Renamed", base_skill: null },
    }));
  });

  it("renders organization consent and sends the PUT learning consent mutation", () => {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals: [] },
      status: { data: { enabled: true, org_owned: true, opted_in: false, publication_blocked: "owner_binding_unavailable" } },
      consent: { mutate: mocks.consent, isPending: false },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      edit: { mutate: mocks.edit, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText(/no owner binding.*only the bound owner can publish/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: /consent to learning/i }));
    expect(mocks.consent).toHaveBeenCalledWith(true);
  });

  it("renders validation diagnostics at their field and line", async () => {
    mocks.feature.mockReturnValue(true);
    const error = new ApiError(400, {
      error: "validation_error",
      error_code: 1008,
      message: "Skill draft failed validation",
      details: { rule: "required", field: "name", line: 3 },
    });
    mocks.review.mockReturnValue({
      data: { proposals: [proposal] },
      status: { data: { enabled: true } },
      edit: { mutate: mocks.edit, error, variables: { proposalId: "proposal" }, isPending: false },
      validate: { mutateAsync: mocks.validate, isPending: false },
      configure: { mutate: mocks.configure, isPending: false },
      approve: { mutateAsync: mocks.approve, isPending: false },
      reject: { mutateAsync: mocks.reject, isPending: false },
      run: { mutate: mocks.run, isPending: false },
      consent: { mutate: mocks.consent, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    fireEvent.click(screen.getByRole("button", { name: "Review draft" }));
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Skill name" })).toHaveAccessibleDescription(
        "Line 3: Fill in the skill name.",
      ),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Skill draft failed validation");
  });

  it("renders the published version on status rows", () => {
    mocks.feature.mockReturnValue(true);
    mocks.review.mockReturnValue({
      data: { proposals: [{ ...proposal, status: "published_unpinned", published_version: "1.2.0" }] },
      status: { data: { enabled: true } },
      configure: { mutate: mocks.configure, isPending: false }, approve: { mutateAsync: mocks.approve, isPending: false }, edit: { mutate: mocks.edit, isPending: false }, reject: { mutateAsync: mocks.reject, isPending: false }, run: { mutate: mocks.run, isPending: false }, consent: { mutate: mocks.consent, isPending: false },
    });
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByText("Published version: 1.2.0")).toBeInTheDocument();
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

  it.each(["learned", "authored"])("shows truthful correction guidance for %s proposals", (source) => {
    withProposals([{ ...proposal, source, status: "publication_failed", failure_code: "ornn_validation_failed" }]);
    render(<AgentLearningReview agentId="agent" />);
    if (source === "authored") {
      expect(screen.getByText(/The agent that drafted it was asked for a corrected draft/)).toBeVisible();
    } else {
      expect(screen.getByText(/Deny or discard this draft and ask NyxBot for a revised one/)).toBeVisible();
      expect(screen.queryByText(/The agent that drafted it was asked/)).toBeNull();
    }
  });

  it("links an authored proposal to its conversation card and shows its state", () => {
    withProposals([{ ...proposal, source: "authored", status: "publication_failed", failure_code: "publish_uncertain", card_conversation_id: "conversation-1", acknowledgement_id: "12345678-1234-4123-8123-123456789012" }]);
    render(<AgentLearningReview agentId="agent" />);
    expect(screen.getByRole("link", { name: "Open the confirmation card" })).toHaveAttribute("href", "/assistant?c=conversation-1&card=12345678-1234-4123-8123-123456789012");
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

it("opens the linked automatically published proposal for inspection and discards its exact attachment", async () => {
  mocks.feature.mockReturnValue(true);
  const previous = mocks.review();
  mocks.review.mockReturnValue({ ...previous, data: { proposals: [{ ...proposal, source: "authored", status: "pinned", auto_allowed: true }] } });
  mocks.json.mockResolvedValue({
    id: "proposal", agent_id: "agent", agent_name: "Operations", revision: 2,
    skills_revision: 3, current_skills_revision: 3, status: "pinned",
    name: "weekly-review", version: "1.0", files: [],
    scope: { status: "reduced", omissions: ["Archived notes are omitted"] },
  });
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
      <AgentLearningReview agentId="agent" proposalId="proposal" />
    </QueryClientProvider>,
  );
  expect(await screen.findByRole("list", { name: "Declared omissions" })).toHaveTextContent("Archived notes are omitted");
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Skill draft" })).toHaveValue(proposal.draft.skill_md);
  expect(screen.getByRole("textbox", { name: "Skill draft" })).toHaveAttribute("readonly");
  expect(screen.queryByText("Confirm this authored skill from its conversation card.")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Discard and detach" }));
  await waitFor(() => expect(mocks.reject).toHaveBeenCalledWith({ proposalId: "proposal", revision: 2 }));
});
