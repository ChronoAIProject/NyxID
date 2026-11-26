import { localizedError } from "../../i18n/localizedError";
import { translateApp } from "../../i18n/appTranslation";
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
export type SkillChange =
  | {
      type: "attach";
      skillId: string;
      selected: SkillVersion;
      preview: SkillPreview;
    }
  | { type: "detach"; skillId: string };

/** An exact package selection and a server revision fence are both required. */
export function buildSkillSelection(current: AgentSkills, change: SkillChange) {
  const existing = current.skills.find(
    (pin) => pin.skill_id === change.skillId,
  );
  if (change.type === "attach") {
    if (existing?.disabled)
      throw localizedError("Enable this skill before updating its exact version. Updating a disabled pin is not supported.");
    const reference = change.preview.reference;
    if (
      reference.skill_id !== change.skillId ||
      reference.version !== change.selected.version ||
      reference.sha256 !== change.selected.sha256
    ) {
      throw localizedError("The package does not match the selected skill and exact version. Reload its versions and try again.");
    }
    if (!existing && current.skills.length >= 16)
      throw localizedError("This agent already has 16 skills. Detach one first.");
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
    throw localizedError("This skill is no longer attached. Reload the agent's skills.");
  return {
    expected_revision: current.revision,
    skills: current.skills.filter((pin) => pin.skill_id !== change.skillId),
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
    throw localizedError("These skills changed elsewhere. Reload and review the current selection before saving.");
}

export const SKILL_HISTORY_LABELS: Record<string, string> = {
  get assistant_skills_updated() {
    return translateApp("Attached skills changed");
  },
  get assistant_learning_proposal_edited() {
    return translateApp("Skill draft edited");
  },
  get assistant_learning_proposal_rejected() {
    return translateApp("Skill draft rejected");
  },
  get assistant_learning_proposal_reprepared() {
    return translateApp("Skill draft package rebuilt");
  },
  get assistant_learning_publication_approved() {
    return translateApp("Skill publication approved");
  },
  get assistant_learning_publication_deferred() {
    return translateApp("Skill publication deferred");
  },
  get assistant_learning_skill_pinned() {
    return translateApp("Published skill attached");
  },
};
