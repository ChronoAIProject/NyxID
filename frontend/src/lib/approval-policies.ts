import type {
  ApprovalEffect,
  ApprovalMode,
  ApprovalRule,
  ApprovalVerb,
  ServiceApprovalConfigItem,
} from "@/types/approvals";

export const approvalVerbs = ["read", "write", "destructive"] as const;

export const approvalVerbLabels: Record<ApprovalVerb, string> = {
  read: "Read",
  write: "Write",
  destructive: "Destructive",
};

export function buildSimpleApprovalRules(
  enabledVerbs: readonly ApprovalVerb[],
  mode: ApprovalMode,
): {
  readonly approvalRequired: boolean;
  readonly defaultEffect: ApprovalEffect;
  readonly rules: readonly ApprovalRule[];
} {
  const enabled = new Set(enabledVerbs);
  const everyVerbEnabled = approvalVerbs.every((verb) => enabled.has(verb));

  if (everyVerbEnabled) {
    return {
      approvalRequired: true,
      defaultEffect: "require_approval",
      rules: [],
    };
  }

  return {
    approvalRequired: enabled.size > 0,
    defaultEffect: "auto_allow",
    rules: approvalVerbs
      .filter((verb) => enabled.has(verb))
      .map((verb) => ({
        methods: [],
        resource_pattern: "*",
        verbs: [verb],
        effect: "require_approval",
        mode,
      })),
  };
}

export function enabledSimpleApprovalVerbs(
  config: ServiceApprovalConfigItem,
): readonly ApprovalVerb[] {
  const defaultRequiresApproval =
    config.default_effect === "require_approval" ||
    (config.default_effect === null &&
      config.rules.length === 0 &&
      config.approval_required);

  if (defaultRequiresApproval) {
    return approvalVerbs;
  }

  const enabled = new Set<ApprovalVerb>();
  for (const rule of config.rules) {
    if (!isSimpleRequireApprovalRule(rule)) continue;
    for (const verb of rule.verbs) {
      enabled.add(verb);
    }
  }

  return approvalVerbs.filter((verb) => enabled.has(verb));
}

export function approvalPolicyLabel(config: ServiceApprovalConfigItem): string {
  const enabled = enabledSimpleApprovalVerbs(config);
  if (config.default_effect === "deny") return "Unmatched operations denied";
  if (enabled.length === approvalVerbs.length) return "Approval required";
  if (enabled.length === 0) return "Approval not required";
  return `Approval for ${enabled.map((verb) => approvalVerbLabels[verb]).join(", ")}`;
}

function isSimpleRequireApprovalRule(rule: ApprovalRule): boolean {
  return (
    rule.effect === "require_approval" &&
    (rule.methods.length === 0 || rule.methods.includes("*")) &&
    (rule.resource_pattern.trim() === "" || rule.resource_pattern.trim() === "*")
  );
}
