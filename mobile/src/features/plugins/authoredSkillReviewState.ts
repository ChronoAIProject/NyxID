import { localizedError } from "../../i18n/localizedError";
import { translateApp } from "../../i18n/appTranslation";
import { z } from "zod";

export const authoredSkillPreviewSchema = z.object({
  id: z.string(),
  agent_id: z.string(),
  agent_name: z.string(),
  revision: z.number().int().nonnegative(),
  skills_revision: z.number().int().nonnegative(),
  current_skills_revision: z.number().int().nonnegative(),
  status: z.string(),
  state: z.string().default("active"),
  actions: z.array(z.string()).default([]),
  name: z.string(),
  version: z.string(),
  files: z.array(z.object({ path: z.string(), content: z.string() })),
  failure_code: z.string().nullish(),
  published_version: z.string().nullish(),
  published_skill_id: z.string().nullish(),
  lease_live: z.boolean().nullish(),
  base_scripts_not_copied: z.boolean().nullish(),
});
export type AuthoredSkillPreview = z.infer<typeof authoredSkillPreviewSchema>;
export type AuthoredSkillReference = {
  agent_id: string;
  proposal_id: string;
  revision?: number;
  skills_revision?: number;
};
export const AUTHORED_SKILL_ACTIONS = [
  { action: "deny",
    get label() {
      return translateApp("Deny");
    },
  },
  { action: "dismiss_card",
    get label() {
      return translateApp("Dismiss confirmation");
    },
  },
  { action: "discard",
    get label() {
      return translateApp("Discard draft");
    },
  },
  { action: "renew",
    get label() {
      return translateApp("Request a new confirmation");
    },
  },
  { action: "publish",
    get label() {
      return translateApp("Allow and publish");
    },
  },
  { action: "retry",
    get label() {
      return translateApp("Retry publication");
    },
  },
  { action: "check",
    get label() {
      return translateApp("Check again");
    },
  },
  { action: "reprepare",
    get label() {
      return translateApp("Review updated package");
    },
  },
  { action: "show_updated_draft",
    get label() {
      return translateApp("Show updated draft");
    },
  },
] as const;
export type AuthoredSkillAction =
  (typeof AUTHORED_SKILL_ACTIONS)[number]["action"];

const REVIEW_STATES = new Set([
  "active",
  "changed",
  "pinned",
  "discarded",
  "invalidated",
  "dismissed",
]);
export function supportedAuthoredSkillActions(
  data: AuthoredSkillPreview,
): AuthoredSkillAction[] {
  if (data.state !== "active" && data.state !== "changed") return [];
  return AUTHORED_SKILL_ACTIONS.flatMap(({ action }) =>
    data.actions.includes(action) &&
    (data.state === "active" || action === "show_updated_draft")
      ? [action]
      : [],
  );
}

export function authoredReviewMatches(
  reference: AuthoredSkillReference,
  data: AuthoredSkillPreview,
) {
  return (
    REVIEW_STATES.has(data.state) &&
    reference.proposal_id === data.id &&
    reference.agent_id === data.agent_id &&
    typeof reference.revision === "number" &&
    typeof reference.skills_revision === "number" &&
    // The preview's skills_revision is the publication's prior attempt
    // snapshot. A renewed card may bind a newer skills revision; the server's
    // card-bound action list is authoritative for that revision fence.
    (data.state !== "active" || reference.revision === data.revision)
  );
}

/** Only the server's advertised action can admit a write or acknowledgement. */
export function authoredSkillActionRequest(
  reference: AuthoredSkillReference,
  data: AuthoredSkillPreview,
  acknowledgementId: string,
  action: AuthoredSkillAction,
):
  | { kind: "decision"; choice: "allow" | "deny" }
  | {
      kind: "post";
      action: "approve" | "reject" | "reprepare";
      body: Record<string, unknown>;
    } {
  if (!authoredReviewMatches(reference, data))
    throw localizedError("This confirmation does not match the reviewed draft. Reload it before continuing.");
  if (!supportedAuthoredSkillActions(data).includes(action))
    throw localizedError("This action is no longer available. Reload the current publication state.");
  if (action === "deny" || action === "dismiss_card")
    return { kind: "decision", choice: "deny" };
  if (action === "publish" || action === "retry" || action === "check")
    return { kind: "decision", choice: "allow" };
  if (action === "discard")
    return {
      kind: "post",
      action: "reject",
      body: { revision: reference.revision, reason: "rejected" },
    };
  if (action === "renew")
    return {
      kind: "post",
      action: "approve",
      body: {
        revision: reference.revision,
        agent_skills_revision: data.current_skills_revision,
        renewal_of: acknowledgementId,
      },
    };
  return {
    kind: "post",
    action: "reprepare",
    body: { acknowledgement_id: acknowledgementId },
  };
}

export function authoredSkillStateLabel(state: string) {
  return state === "pinned"
    ? translateApp("Skill published and attached")
    : state === "discarded"
      ? translateApp("Skill draft discarded")
      : state === "invalidated"
        ? translateApp("Skill draft is no longer available")
        : state === "dismissed"
          ? translateApp(
              "Confirmation dismissed; the publication remains available.",
            )
          : translateApp("This confirmation no longer matches the draft.");
}
export function interruptedPublicationText(actions: readonly string[]) {
  if (
    ["publish", "retry", "deny", "discard"].some((action) =>
      actions.includes(action),
    )
  )
    return translateApp(
      "The last attempt was interrupted before anything was sent to Ornn.",
    );
  return actions.includes("check")
    ? translateApp(
        "The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version. Check again only reads; it never writes again.",
      )
    : translateApp(
        "The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version.",
      );
}

const failureCopy: Record<string, string> = {
  get base_changed() {
    return translateApp(
      "The source skill changed. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get base_verify_failed() {
    return translateApp(
      "The source skill is no longer the reviewed private version. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get base_interface_incompatible() {
    return translateApp(
      "The source skill interface cannot be preserved. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_validation_failed() {
    return translateApp(
      "Ornn rejected the package format. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_interface_change_requires_major() {
    return translateApp(
      "Ornn requires a major version for this interface change. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_write_forbidden() {
    return translateApp("Ornn update permission or skill write access denied.");
  },
  get ornn_read_forbidden() {
    return translateApp(
      "Ornn read access is denied. Check your Ornn permissions and try again.",
    );
  },
  get ornn_auth_missing() {
    return translateApp("Connect your Ornn access and try again.");
  },
  get ornn_skill_not_found() {
    return translateApp(
      "The source skill is unavailable through your Ornn access.",
    );
  },
  get ornn_package_rejected() {
    return translateApp(
      "Ornn rejected the skill package. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_dependency_rejected() {
    return translateApp(
      "Ornn rejected a skill dependency. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_name_taken() {
    return translateApp(
      "That skill name is already in use. Deny or discard this draft and ask NyxBot for a revised one.",
    );
  },
  get ornn_unavailable() {
    return translateApp("Ornn is unavailable. Try again later.");
  },
  get nyxid_refused() {
    return translateApp(
      "Publication is temporarily unavailable. Try again later.",
    );
  },
  get publish_uncertain() {
    return translateApp(
      "NyxID cannot yet confirm whether Ornn published this version; nothing else will be written for it.",
    );
  },
  get version_conflict() {
    return translateApp(
      "This version exists with different content. NyxID did not attach it. Ask NyxBot for a new draft after this publication is resolved.",
    );
  },
  get verify_failed() {
    return translateApp(
      "NyxID could not verify the exact private version. Check again.",
    );
  },
  get pin_conflict() {
    return translateApp(
      "The version was verified but could not be attached. Check again.",
    );
  },
  get approval_expired() {
    return translateApp(
      "This confirmation expired. Request a new confirmation for the same draft.",
    );
  },
  get target_busy() {
    return translateApp(
      "Another draft is publishing this skill version. Wait for it, or discard it first.",
    );
  },
  get operator_released() {
    return translateApp(
      "An operator confirmed the earlier attempt had no effect. Publish this draft again or discard it.",
    );
  },
  get evidence_unavailable() {
    return translateApp(
      "Learning evidence or consent is no longer available, so this draft cannot be attached.",
    );
  },
  get draft_unavailable() {
    return translateApp(
      "The draft files are unavailable. Publication cannot continue.",
    );
  },
  get legacy_package_requires_reprepare() {
    return translateApp(
      "This draft was prepared by an older NyxID version that cannot preserve the skill's tools and runtimes. Review the updated package.",
    );
  },
};
export function publicationFailureText(code: string, status: string) {
  if (
    status === "published_unpinned" &&
    (code === "base_changed" || code === "evidence_unavailable")
  )
    return (code === "base_changed" ? translateApp("NyxID verified this private version in Ornn but did not attach it because the source skill changed. Attach it from the agent's skills if you still want it.") : translateApp("NyxID verified this private version in Ornn but did not attach it because learning evidence or consent was withdrawn. Attach it from the agent's skills if you still want it."));
  return (
    failureCopy[code] ??
    translateApp("Publication could not finish. Check the current state.")
  );
}
