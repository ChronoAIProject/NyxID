import { z } from "zod";
const pinSchema = z.object({
  source: z.literal("ornn"),
  skill_id: z.string(),
  name: z.string(),
  version: z.string(),
  sha256: z.string(),
});
/** An exact Ornn pin: the shape a draft's `base_skill` carries. */
export const skillPinSchema = pinSchema;
export type SkillPin = z.infer<typeof skillPinSchema>;
export const agentSkillSchema = pinSchema.extend({
  dependencies: pinSchema.array().default([]),
  disabled: z.boolean().default(false),
});
export type AgentSkill = z.infer<typeof agentSkillSchema>;
export const agentSkillsSchema = z.object({
  revision: z.number(),
  skills: agentSkillSchema.array(),
  metadata: z.record(
    z.string(),
    z.object({
      description: z.string(),
      size_bytes: z.number(),
      preload_paths: z.array(z.string()).default([]),
      preload_bytes: z.number().default(0),
    }),
  ),
});
export const skillPreloadFormSchema = z
  .object({
    enabled: z.boolean(),
    paths: z.string().max(2048),
  })
  .superRefine((value, context) => {
    if (!value.enabled) return;
    const paths = value.paths
      .split("\n")
      .map((path) => path.trim())
      .filter(Boolean);
    let message: string | undefined;
    if (paths.length > 8 || !paths.includes("SKILL.md")) {
      message = "Include SKILL.md and choose at most 8 files.";
    } else if (
      new Set(paths).size !== paths.length ||
      paths.some(
        (path) =>
          path.length > 256 ||
          path.includes("\\") ||
          path.includes(":") ||
          Array.from(path).some((character) => {
            const code = character.codePointAt(0)!;
            return code < 32 || (code >= 127 && code <= 159);
          }) ||
          !/\.(md|txt)$/.test(path) ||
          path
            .split("/")
            .some((part) => !part || part === "." || part === ".."),
      )
    ) {
      message = "Use unique relative .md or .txt paths, one per line.";
    }
    if (message) context.addIssue({ code: "custom", path: ["paths"], message });
  });
export const skillSearchSchema = z.object({
  items: z
    .object({ id: z.string(), name: z.string(), description: z.string() })
    .array(),
  page: z.number(),
  total_pages: z.number(),
});
export const skillVersionsSchema = z.object({
  page: z.number().default(1),
  total_pages: z.number().default(1),
  items: z
    .object({
      version: z.string(),
      sha256: z.string(),
      deprecated: z.boolean(),
    })
    .array(),
});
export const skillPreviewSchema = z.object({
  reference: agentSkillSchema,
  description: z.string(),
  size_bytes: z.number(),
});

export const authoredSkillPreviewSchema = z.object({
  id: z.string(),
  agent_id: z.string(),
  agent_name: z.string(),
  revision: z.number(),
  skills_revision: z.number(),
  current_skills_revision: z.number(),
  status: z.string(),
  state: z.string().default("active"),
  // Unknown actions from a newer server are ignored rather than failing the card.
  actions: z.array(z.string()).default([]),
  name: z.string(),
  version: z.string(),
  files: z.array(z.object({ path: z.string(), content: z.string() })),
  failure_code: z.string().nullish(),
  failure_rules: z.array(z.string()).default([]),
  failure_details_unavailable: z.boolean().default(false),
  failure_stage: z.string().nullish(),
  coverage: z.array(z.object({ id: z.string(), name: z.string(), expired: z.boolean(), unknown: z.boolean().default(false), reads: z.number(), percent: z.number(), lower_bound: z.boolean(), since_reset_percent: z.number().nullish() })).default([]),
  coverage_omitted: z.number().default(0),
  scope_unavailable: z.boolean().default(false),
  scope: z.object({ status: z.enum(["complete", "reduced"]), omissions: z.array(z.string()) }).nullish(),
  published_version: z.string().nullish(),
  published_skill_id: z.string().nullish(),
  lease_live: z.boolean().nullish(),
  base_scripts_not_copied: z.boolean().nullish(),
});

export const learningProposalSchema = z.object({
  source: z.enum(["learned", "authored"]).default("learned"),
  id: z.string(),
  agent_id: z.string(),
  status: z.string(),
  revision: z.number().default(0),
  config_revision: z.number(),
  agent_skills_revision: z.number(),
  evidence_count: z.number(),
  body_bytes: z.number(),
  created_at: z.string(),
  updated_at: z.string(),
  failure_code: z.string().nullish(),
  failure_rules: z.array(z.string()).default([]),
  failure_stage: z.string().nullish(),
  published_skill_id: z.string().nullish(),
  published_version: z.string().nullish(),
  // Older servers omit it; their listed proposals always had current evidence.
  evidence_available: z.boolean().default(true),
  auto_allowed: z.boolean().default(false),
  card_conversation_id: z.string().nullish(),
  acknowledgement_id: z.string().nullish(),
  card_status: z.string().nullish(),
  card_expires_at: z.string().nullish(),
  review_url: z.string().nullish(),
  draft: z
    .object({
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
      // A malformed pin from an older server must not hide the proposal.
      base_skill: skillPinSchema.nullish().catch(null),
    })
    .nullish(),
});
export const learningProposalsSchema = z.object({
  proposals: learningProposalSchema.array(),
});
export const learningStatusSchema = z.object({
  available: z.boolean().optional(),
  can_maintain: z.boolean().optional(),
  org_owned: z.boolean().optional(),
  enabled: z.boolean(),
  threshold: z.number().optional(),
  learning_epoch: z.number().optional(),
  config_revision: z.number().optional(),
  eligible_count: z.number().optional(),
  last_run_at: z.string().nullable().optional(),
  last_error_code: z.string().nullable().optional(),
  opted_in: z.boolean().optional(),
  publication_blocked: z.string().nullable().optional(),
});
export const learningApprovalSchema = z.object({
  status: z.string(),
  acknowledgement: z.object({ acknowledgement_id: z.string() }).optional(),
});

/** `GET /agents/{id}/skills/{skill_id}/preview`: the pinned version as the agent reads it. */
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
export type AttachedSkillPreview = z.infer<typeof attachedSkillPreviewSchema>;

export const agentSkillHistoryItemSchema = z.object({
  id: z.string(),
  event_type: z.string(),
  created_at: z.string(),
  revision: z.number().nullable().default(null),
  proposal_id: z.string().nullable().default(null),
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
});
export type AgentSkillHistoryItem = z.infer<typeof agentSkillHistoryItemSchema>;
export const agentSkillHistorySchema = z.object({
  items: z.array(agentSkillHistoryItemSchema).default([]),
});

/** `POST /agents/{id}/skills/{skill_id}/improve`. */
export const skillImproveSchema = z.object({
  proposal_id: z.string(),
  acknowledgement_id: z.string(),
});

/** `POST /learning/proposals/{pid}/validate` success. */
export const skillDraftValidationSchema = z.object({
  valid: z.boolean(),
  bytes: z.number(),
});

/** One field diagnostic in a 400 `validation_error` `details`. */
export const skillDraftDiagnosticSchema = z.object({
  rule: z.string(),
  field: z.string(),
  line: z.number().optional(),
  limit: z.number().optional(),
  actual: z.number().optional(),
  unit: z.string().optional(),
});
export type SkillDraftDiagnostic = z.infer<typeof skillDraftDiagnosticSchema> & {
  /** How to fix it, derived from the rule by `skillDraftRemediation`. */
  readonly remediation: string;
};

const DRAFT_FIELD_LABELS: Record<string, string> = {
  name: "the skill name",
  description: "the description",
  skill_md: "SKILL.md",
  rationale: "the rationale",
  safety_notes: "the safety notes",
  kind: "the draft kind",
  base_skill: "the base skill",
  files: "the files",
  draft: "the draft",
  schema_version: "the draft format",
};

/** A readable name for a diagnostic's field, including `files[i].path`. */
export function skillDraftFieldLabel(field: string): string {
  const file = /^files\[(\d+)\]\.(path|content)$/.exec(field);
  if (file) {
    return `file ${Number(file[1]) + 1} ${file[2] === "path" ? "path" : "content"}`;
  }
  return DRAFT_FIELD_LABELS[field] ?? field;
}

/**
 * How to fix one server diagnostic. The server sends only the rule and its
 * measurements, so this is the single rule-to-remediation map.
 */
export function skillDraftRemediation(
  diagnostic: z.infer<typeof skillDraftDiagnosticSchema>,
): string {
  const field = skillDraftFieldLabel(diagnostic.field);
  const at = diagnostic.line ? ` on line ${diagnostic.line}` : "";
  switch (diagnostic.rule) {
    case "required":
      return `Fill in ${field}.`;
    case "too_large": {
      if (diagnostic.limit === undefined) return `Shorten ${field}.`;
      const unit = diagnostic.unit ? ` ${diagnostic.unit}` : "";
      const actual =
        diagnostic.actual === undefined
          ? ""
          : ` (it has ${diagnostic.actual.toLocaleString("en-US")})`;
      return `Shorten ${field} to at most ${diagnostic.limit.toLocaleString("en-US")}${unit}${actual}.`;
    }
    case "credential_shape":
      return `Remove the credential-like text${at} in ${field}; skills must never contain secrets.`;
    case "text_only":
      return `Use plain text${at} in ${field}; remove binary or control characters.`;
    case "invalid_name":
      return "Use lowercase letters, digits and single hyphens, with no hyphen at the start or end.";
    case "invalid_kind":
      return 'Choose "new" or "improve".';
    case "base_skill_mismatch":
      return "An improve draft needs a base skill; a new skill must not have one.";
    case "base_skill_not_attached":
      return "Pick one of this agent's attached skills as the base skill.";
    case "base_skill_name_mismatch":
      return "An improve draft keeps the base skill's name; restore that name.";
    case "unsafe_path":
      return "Use a relative path ending in .md or .txt, without a leading /, .., \\ or :.";
    case "duplicate_path":
      return "Give every file a unique path; SKILL.md is reserved.";
    case "invalid_shape":
      return `Correct the format of ${field}.`;
    case "unknown_field":
      return `Remove ${field}; this draft format does not support it.`;
    case "unsupported_schema":
      return "This draft uses a format this page cannot edit. Reload the draft and try again.";
    case "private_material":
      return `Remove emails, tokens, keys or other private data from ${field}.`;
    default:
      return `Check ${field} and try again.`;
  }
}

/** Diagnostics from a request error; anything that is not one is dropped. */
export function skillDraftDiagnostics(cause: unknown): SkillDraftDiagnostic[] {
  const details = (cause as { errorResponse?: { details?: unknown } } | null)
    ?.errorResponse?.details;
  const candidates = Array.isArray(details)
    ? details
    : details === undefined || details === null
      ? []
      : [details];
  return candidates.flatMap((item) => {
    const parsed = skillDraftDiagnosticSchema.safeParse(item);
    return parsed.success
      ? [{ ...parsed.data, remediation: skillDraftRemediation(parsed.data) }]
      : [];
  });
}

/** Only these drafts accept edits; the server refuses the rest. */
export function learningDraftEditable(status: string): boolean {
  return status === "pending" || status === "publication_failed";
}

/** The draft editor's form. `base_skill_id` selects one attached pin. */
export const skillDraftFormSchema = z
  .object({
    kind: z.enum(["new", "improve", "none"]),
    name: z.string().trim().min(1, "Enter a skill name."),
    description: z.string(),
    skill_md: z.string().min(1, "SKILL.md cannot be empty."),
    rationale: z.string(),
    safety_notes: z.string(),
    base_skill_id: z.string(),
    files: z.array(
      z.object({
        path: z.string().trim().min(1, "Enter a file path or remove the file."),
        content: z.string(),
      }),
    ),
  })
  .superRefine((value, ctx) => {
    if (value.kind === "improve" && !value.base_skill_id) {
      ctx.addIssue({
        code: "custom",
        path: ["base_skill_id"],
        message: "Choose the attached skill this draft improves.",
      });
    }
  });
export type SkillDraftForm = z.infer<typeof skillDraftFormSchema>;
