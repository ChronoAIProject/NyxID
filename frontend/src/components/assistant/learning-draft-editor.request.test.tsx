import type { ComponentProps } from "react";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentLearningReview } from "./agent-learning-review";

// The real hooks, schemas and request path (assistantJson -> fetch -> ApiError);
// only fetch, the feature flag, routing and the signed-in user are stubbed.
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, ...props }: ComponentProps<"a"> & { to: string; search?: Record<string, string> }) =>
    <a href={to + (search ? `?${new URLSearchParams(search)}` : "")} {...props} />,
}));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (s: unknown) => unknown) => select({ user: { id: "owner" } }) }));

const api = "/api/v1/assistant/nyxagent/agents/agent";
const attachedPin = {
  source: "ornn", skill_id: "skill-1", name: "weekly-review", version: "1.2.0", sha256: "a".repeat(64),
  dependencies: [{ source: "ornn", skill_id: "dep", name: "dep", version: "1.0.0", sha256: "b".repeat(64) }],
};
const draft = {
  schema_version: 1, kind: "new", name: "weekly-review", description: "A reusable procedure",
  skill_md: "# Weekly review", rationale: "Reusable", safety_notes: "Review before use",
  files: [{ path: "notes.md", content: "Checklist" }], base_skill: null,
};
let proposal: Record<string, unknown>;
let errors: Array<{ status: number; body: unknown }>;
let validateFailures: Array<{ status: number; body: unknown }>;
let fetchMock: ReturnType<typeof vi.fn>;

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

beforeEach(() => {
  globalThis.__nyxidAssistantHttpMock = undefined;
  proposal = {
    source: "learned", id: "proposal", agent_id: "agent", status: "pending", revision: 2, config_revision: 1,
    agent_skills_revision: 3, evidence_count: 2, body_bytes: 42, created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z", draft,
  };
  errors = [];
  validateFailures = [];
  fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    if (url === `${api}/learning/proposals/proposal` && init?.method === "PUT") {
      const failure = errors.shift();
      if (failure) return json(failure.body, failure.status);
      const sent = JSON.parse(String(init.body)) as { draft: Record<string, unknown> };
      proposal = { ...proposal, revision: 3, draft: sent.draft };
      return json({ status: "updated" });
    }
    if (url === `${api}/learning/proposals/proposal/validate`) {
      const failure = validateFailures.shift();
      return failure ? json(failure.body, failure.status) : json({ valid: true, bytes: 1234 });
    }
    if (url === `${api}/learning/proposals`) return json({ proposals: [proposal] });
    if (url === `${api}/skills`) return json({ agent_id: "agent", revision: 3, skills: [attachedPin], metadata: {} });
    return json({ enabled: true });
  });
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function show() {
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })}>
    <AgentLearningReview agentId="agent" />
  </QueryClientProvider>);
}

function sentDrafts(): Array<Record<string, unknown>> {
  return fetchMock.mock.calls
    .filter(([url, init]) => url === `${api}/learning/proposals/proposal` && (init as RequestInit | undefined)?.method === "PUT")
    .map(([, init]) => JSON.parse(String((init as RequestInit).body)) as Record<string, unknown>);
}

function validationError(details: Record<string, unknown>) {
  return { status: 400, body: { error: "validation_error", error_code: 1008, message: "Skill draft failed validation", details } };
}

async function openEditor() {
  const user = userEvent.setup();
  show();
  await user.click(await screen.findByRole("button", { name: "Review draft" }));
  return user;
}

it("saves the complete draft, including an unchanged SKILL.md", async () => {
  const user = await openEditor();
  const name = screen.getByRole("textbox", { name: "Skill name" });
  await user.clear(name);
  await user.type(name, "weekly-summary");
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(sentDrafts()).toHaveLength(1));
  expect(sentDrafts()[0]).toEqual({
    revision: 2,
    draft: { ...draft, name: "weekly-summary" },
  });
});

it("renders each server diagnostic beside the input it names, with its remediation", async () => {
  errors = [
    validationError({ rule: "invalid_name", field: "name" }),
    validationError({ rule: "unsafe_path", field: "files[0].path" }),
    validationError({ rule: "too_large", field: "skill_md", limit: 20000, actual: 20480, unit: "characters" }),
    validationError({ rule: "unsupported_schema", field: "schema_version" }),
  ];
  const user = await openEditor();
  const name = screen.getByRole("textbox", { name: "Skill name" });
  await user.type(name, "!");
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(name).toHaveAccessibleDescription(
    "Use lowercase letters, digits and single hyphens, with no hyphen at the start or end.",
  ));
  expect(name).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("alert")).toHaveTextContent("Skill draft failed validation");

  await user.click(screen.getByRole("button", { name: "Save draft" }));
  const path = screen.getByRole("textbox", { name: "File 1 path" });
  await waitFor(() => expect(path).toHaveAccessibleDescription(
    "Use a relative path ending in .md or .txt, without a leading /, .., \\ or :.",
  ));
  // A fresh attempt clears the previous field's diagnostic.
  expect(name).not.toHaveAccessibleDescription(/lowercase letters/);

  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(screen.getByRole("textbox", { name: "Skill draft" })).toHaveAccessibleDescription(
    "Shorten SKILL.md to at most 20,000 characters (it has 20,480).",
  ));

  // Draft-level rules have no input of their own and go to the form banner.
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent(
    "the draft format: This draft uses a format this page cannot edit. Reload the draft and try again.",
  ));
});

it("sends an improve draft's base skill as the selected attached SkillPin", async () => {
  errors = [validationError({ rule: "base_skill_not_attached", field: "base_skill" })];
  const user = await openEditor();
  await user.click(screen.getByRole("combobox", { name: "Skill kind" }));
  await user.click(screen.getByRole("option", { name: "Improve an attached skill" }));
  const base = await screen.findByRole("combobox", { name: "Base skill" });
  await user.click(base);
  await user.click(await screen.findByRole("option", { name: "weekly-review · 1.2.0" }));
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(base).toHaveAccessibleDescription(
    "Pick one of this agent's attached skills as the base skill.",
  ));
  expect(sentDrafts()[0]).toEqual({
    revision: 2,
    draft: {
      ...draft,
      kind: "improve",
      base_skill: { source: "ornn", skill_id: "skill-1", name: "weekly-review", version: "1.2.0", sha256: "a".repeat(64) },
    },
  });
});

it("refuses an improve draft without a base skill before sending it", async () => {
  const user = await openEditor();
  await user.click(screen.getByRole("combobox", { name: "Skill kind" }));
  await user.click(screen.getByRole("option", { name: "Improve an attached skill" }));
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  expect(await screen.findByRole("combobox", { name: "Base skill" })).toHaveAccessibleDescription(
    "Choose the attached skill this draft improves.",
  );
  expect(sentDrafts()).toHaveLength(0);
});

it("keeps a file row mounted and focused while its path is edited", async () => {
  const user = await openEditor();
  const path = screen.getByRole("textbox", { name: "File 1 path" });
  await user.click(path);
  await user.type(path, "x");
  await user.type(path, "y");
  expect(screen.getByRole("textbox", { name: "File 1 path" })).toBe(path);
  expect(path).toHaveFocus();
  expect(path).toHaveValue("notes.mdxy");
});

it("starts from the saved revision after a successful save", async () => {
  const user = await openEditor();
  const description = screen.getByRole("textbox", { name: "Skill description" });
  await user.clear(description);
  await user.type(description, "Saved description");
  await user.click(screen.getByRole("button", { name: "Add file" }));
  await user.type(screen.getByRole("textbox", { name: "File 2 path" }), "more.md");
  await user.click(screen.getByRole("button", { name: "Save draft" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Save draft" })).not.toBeInTheDocument());
  const form = screen.getByRole("form", { name: "Edit skill draft" });
  expect(within(form).getByRole("textbox", { name: "Skill description" })).toHaveValue("Saved description");
  expect(within(form).getByRole("textbox", { name: "File 2 path" })).toHaveValue("more.md");
  expect(sentDrafts()).toHaveLength(1);
});

function validated(): Array<Record<string, unknown>> {
  return fetchMock.mock.calls
    .filter(([url]) => url === `${api}/learning/proposals/proposal/validate`)
    .map(([, init]) => JSON.parse(String((init as RequestInit).body)) as Record<string, unknown>);
}

it("validates the current values without saving them", async () => {
  const user = await openEditor();
  const description = screen.getByRole("textbox", { name: "Skill description" });
  await user.clear(description);
  await user.type(description, "Checked");
  await user.click(screen.getByRole("button", { name: "Validate" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "The draft passes validation (1,234 bytes). Save it to keep these changes.",
  );
  expect(validated()).toEqual([{ draft: { ...draft, description: "Checked" } }]);
  expect(sentDrafts()).toHaveLength(0);
  // Any later edit makes the result stale.
  await user.type(description, "!");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("shows validation diagnostics beside their fields and a gone proposal in the banner", async () => {
  validateFailures = [
    validationError({ rule: "too_large", field: "description", limit: 1024, actual: 2000, unit: "characters" }),
    { status: 404, body: { error: "not_found", error_code: 1004, message: "Learning proposal not found" } },
  ];
  const user = await openEditor();
  await user.click(screen.getByRole("button", { name: "Validate" }));
  await waitFor(() => expect(screen.getByRole("textbox", { name: "Skill description" })).toHaveAccessibleDescription(
    "Shorten the description to at most 1,024 characters (it has 2,000).",
  ));
  expect(screen.getByRole("alert")).toHaveTextContent("Skill draft failed validation");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Validate" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Learning proposal not found"));
  expect(screen.getByRole("textbox", { name: "Skill description" })).not.toHaveAccessibleDescription(/Shorten/);
});
