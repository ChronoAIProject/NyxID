import { z } from "zod";

// Preserve additive server fields (for example activation metadata) when a
// whole selection is written back. A sibling pin must never be normalized away.
export const skillPinSchema = z
  .object({
    source: z.literal("ornn"),
    skill_id: z.string(),
    name: z.string(),
    version: z.string(),
    sha256: z.string(),
  })
  .passthrough();
export const agentSkillSchema = skillPinSchema.extend({
  dependencies: z.array(skillPinSchema).default([]),
  disabled: z.boolean().default(false),
});
export type AgentSkill = z.infer<typeof agentSkillSchema>;
export const agentSkillsSchema = z.object({
  agent_id: z.string().optional(),
  revision: z.number().int().nonnegative(),
  skills: z.array(agentSkillSchema),
  metadata: z.record(
    z.string(),
    z.object({
      description: z.string(),
      size_bytes: z.number(),
    }),
  ),
});
export type AgentSkills = z.infer<typeof agentSkillsSchema>;
export const skillSearchSchema = z.object({
  items: z.array(
    z.object({ id: z.string(), name: z.string(), description: z.string() }),
  ),
  page: z.number(),
  total_pages: z.number(),
});
export type CatalogSkill = z.infer<typeof skillSearchSchema>["items"][number];
export const skillVersionSchema = z.object({
  version: z.string(),
  sha256: z.string(),
  deprecated: z.boolean(),
});
export type SkillVersion = z.infer<typeof skillVersionSchema>;
export const skillVersionsSchema = z.object({
  items: z.array(skillVersionSchema),
  page: z.number().default(1),
  total_pages: z.number().default(1),
});
export const skillPreviewSchema = z.object({
  reference: agentSkillSchema,
  description: z.string(),
  size_bytes: z.number(),
});
export type SkillPreview = z.infer<typeof skillPreviewSchema>;
export const attachedSkillPreviewSchema = z.object({
  skill_id: z.string(),
  name: z.string(),
  version: z.string(),
  sha256: z.string(),
  disabled: z.boolean().default(false),
  description: z.string(),
  size_bytes: z.number(),
  readable: z.boolean(),
  excerpt: z.string().default(""),
});
export const skillHistorySchema = z.object({
  items: z
    .array(
      z.object({
        id: z.string(),
        event_type: z.string(),
        created_at: z.string(),
        revision: z.number().nullable().default(null),
        skills: z
          .array(
            z.object({
              skill_id: z.string(),
              name: z.string(),
              version: z.string(),
              disabled: z.boolean().default(false),
            }),
          )
          .default([]),
      }),
    )
    .default([]),
});
export const skillDraftSchema = z.object({
  schema_version: z.number().default(1),
  kind: z.enum(["new", "improve", "none"]).default("new"),
  name: z.string(),
  description: z.string(),
  skill_md: z.string(),
  rationale: z.string(),
  safety_notes: z.string(),
  files: z
    .array(z.object({ path: z.string(), content: z.string() }))
    .default([]),
  base_skill: skillPinSchema.nullish(),
});
export type SkillDraft = z.infer<typeof skillDraftSchema>;
export const skillProposalsSchema = z.object({
  proposals: z.array(
    z.object({
      id: z.string(),
      revision: z.number().default(0),
      status: z.string(),
      card_conversation_id: z.string().nullish(),
      acknowledgement_id: z.string().nullish(),
      draft: skillDraftSchema.nullish(),
    }),
  ),
});
export type SkillProposal = z.infer<
  typeof skillProposalsSchema
>["proposals"][number];

export type SkillChange =
  | {
      type: "attach";
      skillId: string;
      selected: SkillVersion;
      preview: SkillPreview;
    }
  | { type: "detach"; skillId: string }
  | { type: "toggle"; skillId: string; disabled: boolean };

/** An exact package selection and a server revision fence are both required. */
export function buildSkillSelection(current: AgentSkills, change: SkillChange) {
  const existing = current.skills.find(
    (pin) => pin.skill_id === change.skillId,
  );
  if (change.type === "attach") {
    if (existing?.disabled)
      throw new Error(
        "Enable this skill before updating its exact version. Updating a disabled pin is not supported.",
      );
    const reference = change.preview.reference;
    if (
      reference.skill_id !== change.skillId ||
      reference.version !== change.selected.version ||
      reference.sha256 !== change.selected.sha256
    ) {
      throw new Error(
        "The package does not match the selected skill and exact version. Reload its versions and try again.",
      );
    }
    if (!existing && current.skills.length >= 16)
      throw new Error("This agent already has 16 skills. Detach one first.");
    const replacement = {
      ...existing,
      ...reference,
      ...(existing ? { disabled: existing.disabled } : {}),
    };
    return {
      expected_revision: current.revision,
      skills: [
        ...current.skills.filter((pin) => pin.skill_id !== change.skillId),
        replacement,
      ],
    };
  }
  if (!existing)
    throw new Error(
      "This skill is no longer attached. Reload the agent's skills.",
    );
  return {
    expected_revision: current.revision,
    skills:
      change.type === "detach"
        ? current.skills.filter((pin) => pin.skill_id !== change.skillId)
        : current.skills.map((pin) =>
            pin.skill_id === change.skillId
              ? { ...pin, disabled: change.disabled }
              : pin,
          ),
  };
}

export function assertSkillRevision(
  agentId: string,
  baseline: AgentSkills,
  latest: AgentSkills,
) {
  if (
    (latest.agent_id && latest.agent_id !== agentId) ||
    baseline.revision !== latest.revision
  )
    throw new Error(
      "These skills changed elsewhere. Reload and review the current selection before saving.",
    );
}

export const SKILL_HISTORY_LABELS: Record<string, string> = {
  assistant_skills_updated: "Attached skills changed",
  assistant_learning_proposal_edited: "Skill draft edited",
  assistant_learning_proposal_rejected: "Skill draft rejected",
  assistant_learning_proposal_reprepared: "Skill draft package rebuilt",
  assistant_learning_publication_approved: "Skill publication approved",
  assistant_learning_publication_deferred: "Skill publication deferred",
  assistant_learning_skill_pinned: "Published skill attached",
};
