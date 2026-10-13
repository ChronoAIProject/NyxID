import { describe, expect, it } from "vitest";
import { ApiError } from "@/lib/api-client";
import {
  agentSkillSchema,
  learningProposalSchema,
  skillDraftDiagnostics,
  skillDraftRemediation,
} from "./agent-skills";

describe("agent skill schema", () => {
  it("defaults disabled pins to false and preserves true", () => {
    const base = { source: "ornn", skill_id: "s", name: "Skill", version: "1.0", sha256: "a".repeat(64) };
    expect(agentSkillSchema.parse(base).disabled).toBe(false);
    expect(agentSkillSchema.parse({ ...base, disabled: true }).disabled).toBe(true);
  });
});

describe("skill draft remediation", () => {
  it("builds the fix from the rule and its measurements", () => {
    expect(skillDraftRemediation({ rule: "too_large", field: "files[1].content", limit: 4096, actual: 5000, unit: "bytes" }))
      .toBe("Shorten file 2 content to at most 4,096 bytes (it has 5,000).");
    expect(skillDraftRemediation({ rule: "credential_shape", field: "skill_md", line: 7 }))
      .toBe("Remove the credential-like text on line 7 in SKILL.md; skills must never contain secrets.");
    expect(skillDraftRemediation({ rule: "required", field: "description" })).toBe("Fill in the description.");
    expect(skillDraftRemediation({ rule: "base_skill_name_mismatch", field: "name" }))
      .toBe("An improve draft keeps the base skill's name; restore that name.");
    expect(skillDraftRemediation({ rule: "private_material", field: "rationale" }))
      .toBe("Remove emails, tokens, keys or other private data from the rationale.");
  });

  it("names the field for an unknown rule", () => {
    expect(skillDraftRemediation({ rule: "brand_new_rule", field: "safety_notes" }))
      .toBe("Check the safety notes and try again.");
  });

  it("reads diagnostics from a real validation error response and drops malformed ones", () => {
    const error = new ApiError(400, {
      error: "validation_error",
      error_code: 1008,
      message: "Skill draft failed validation",
      details: { rule: "unsafe_path", field: "files[0].path" },
    });
    expect(skillDraftDiagnostics(error)).toEqual([{
      rule: "unsafe_path",
      field: "files[0].path",
      remediation: "Use a relative path ending in .md or .txt, without a leading /, .., \\ or :.",
    }]);
    expect(skillDraftDiagnostics(new ApiError(400, { error: "validation_error", error_code: 1008, message: "x", details: { field: 3 } }))).toEqual([]);
    expect(skillDraftDiagnostics(new Error("plain"))).toEqual([]);
  });
});

describe("learning proposal schema", () => {
  const proposal = {
    source: "learned", id: "p", agent_id: "a", status: "published_unpinned", revision: 2, config_revision: 1,
    agent_skills_revision: 3, evidence_count: 2, body_bytes: 42, created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };

  it("keeps the published skill id and version the server sends", () => {
    const parsed = learningProposalSchema.parse({ ...proposal, published_skill_id: "skill-9", published_version: "1.0.1" });
    expect(parsed.published_skill_id).toBe("skill-9");
    expect(parsed.published_version).toBe("1.0.1");
  });

  it("keeps a proposal whose base skill is not a pin", () => {
    const draft = { schema_version: 1, kind: "improve", name: "n", description: "d", skill_md: "# n", rationale: "r", safety_notes: "s", files: [], base_skill: "id@1.0" };
    expect(learningProposalSchema.parse({ ...proposal, draft }).draft?.base_skill).toBeNull();
  });
});
