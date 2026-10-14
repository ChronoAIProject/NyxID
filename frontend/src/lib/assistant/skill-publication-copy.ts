// Fixed, sanitized copy for skill publication failure codes. Registry details
// and skill contents never reach the browser.
const failureCopy: Record<string, string> = {
  base_changed: "The source skill changed. Deny or discard this draft and ask NyxBot for a revised one.",
  base_verify_failed: "The source skill is no longer the reviewed private version. Deny or discard this draft and ask NyxBot for a revised one.",
  base_interface_incompatible: "The source skill interface cannot be preserved. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_validation_failed: "Ornn rejected the package format. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_interface_change_requires_major: "Ornn requires a major version for this interface change. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_write_forbidden: "Ornn update permission or skill write access denied.",
  ornn_read_forbidden: "Ornn read access is denied. Check your Ornn permissions and try again.",
  ornn_auth_missing: "Connect your Ornn access and try again.",
  ornn_skill_not_found: "The source skill is unavailable through your Ornn access.",
  ornn_package_rejected: "Ornn rejected the skill package. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_dependency_rejected: "Ornn rejected a skill dependency. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_name_taken: "That skill name is already in use. Deny or discard this draft and ask NyxBot for a revised one.",
  ornn_unavailable: "Ornn is unavailable. Try again later.",
  nyxid_refused: "Publication is temporarily unavailable. Try again later.",
  publish_uncertain: "NyxID cannot yet confirm whether Ornn published this version; nothing else will be written for it.",
  version_conflict: "This version exists with different content, for example because an earlier publication request landed late. NyxID did not attach it. Ask NyxBot for a new draft after this publication is resolved.",
  verify_failed: "NyxID could not verify the exact private version. Check again.",
  pin_conflict: "The version was verified but could not be attached. Check again.",
  approval_expired: "This confirmation expired. Request a new confirmation for the same draft.",
  target_busy: "Another draft is publishing this skill version. Wait for it, or discard it first.",
  operator_released: "An operator confirmed the earlier attempt had no effect. Publish this draft again (request a new confirmation if this one expired) or discard it.",
  evidence_unavailable: "Learning evidence or consent is no longer available, so this draft cannot be attached.",
  draft_unavailable: "The draft files are unavailable. Publication cannot continue.",
  legacy_package_requires_reprepare: "This draft was prepared by an older NyxID version that can't preserve the skill's tools/runtimes. Review the updated package.",
};

const authoredRepairCopy: Record<string, string> = {
  base_changed: "The source skill changed. The agent that drafted it was asked for a corrected draft.",
  base_verify_failed: "The source skill is no longer the reviewed private version. The agent that drafted it was asked for a corrected draft.",
  base_interface_incompatible: "The source skill interface cannot be preserved. The agent that drafted it was asked for a corrected draft.",
  ornn_validation_failed: "Ornn rejected the package format. The agent that drafted it was asked for a corrected draft.",
  ornn_interface_change_requires_major: "Ornn requires a major version for this interface change. The agent that drafted it was asked for a corrected draft.",
  ornn_package_rejected: "Ornn rejected the skill package. The agent that drafted it was asked for a corrected draft.",
  ornn_dependency_rejected: "Ornn rejected a skill dependency. The agent that drafted it was asked for a corrected draft.",
  ornn_name_taken: "That skill name is already in use. The agent that drafted it was asked for a corrected draft.",
};

const notAttached: Record<string, string> = {
  base_changed: "NyxID verified this private version in Ornn but did not attach it because the source skill changed. Attach it from the agent's skills if you still want it.",
  evidence_unavailable: "NyxID verified this private version in Ornn but did not attach it because learning evidence or consent was withdrawn. Attach it from the agent's skills if you still want it.",
};

export function publicationFailureText(
  code: string,
  status?: string,
  source: "authored" | "learned" = "learned",
): string {
  if (status === "published_unpinned" && notAttached[code]) return notAttached[code];
  if (source === "authored" && authoredRepairCopy[code]) return authoredRepairCopy[code];
  return failureCopy[code] ?? "Publication could not finish. Check the current state.";
}

const ruleCopy: Record<string, string> = {
  allowed_root_items: "Keep supporting files under references/.",
  no_readme_md: "README files are not allowed in the package.",
  skill_md_exists: "The package must contain SKILL.md.",
  no_xml_brackets: "Angle brackets are not allowed in skill metadata.",
  description_no_xml: "Angle brackets are not allowed in the description.",
  name_matches_folder: "The package folder and skill name must match.",
  name_no_forbidden_terms: "The skill name contains a reserved provider term.",
  unexpected_error: "Ornn did not provide a usable format diagnostic.",
  frontmatter_name: "The skill name does not meet Ornn's format rules.",
  frontmatter_description: "The description does not meet Ornn's format rules.",
  frontmatter_version: "The version does not meet Ornn's format rules.",
  frontmatter_category: "The category does not meet Ornn's format rules.",
  frontmatter_output_type: "The output type does not meet Ornn's format rules.",
  frontmatter_runtime: "The runtime does not meet Ornn's format rules.",
  frontmatter_tool_list: "The tool list does not meet Ornn's format rules.",
  frontmatter_tag: "The tags do not meet Ornn's format rules.",
  frontmatter_runtime_env_var: "The runtime environment variables do not meet Ornn's format rules.",
  frontmatter_depends_on: "The dependencies do not meet Ornn's format rules.",
  other: "Ornn reported an additional format rule.",
};

export function publicationRuleText(rule: string): string {
  return ruleCopy[rule] ?? ruleCopy.other!;
}
