import type {
  ApprovalEffect,
  ApprovalMode,
  ApprovalRule,
  ApprovalVerb,
  ServiceApprovalConfigItem,
} from "@/types/approvals";

/** What happens to one action. */
export type ActionChoice = "allow" | "ask" | "block";

/** What happens to every action the owner did not set individually. */
export type BaseChoice = "ask_changes" | "ask_all" | "allow_all";

export interface ProtectionAction {
  readonly id: string;
  readonly name: string;
  readonly method: string;
  readonly path: string;
}

export interface ActionSetting {
  readonly choice: ActionChoice;
  /** Only meaningful for `ask`: false lets one approval be remembered. */
  readonly everyTime: boolean;
}

export interface ProtectionDraft {
  readonly base: BaseChoice;
  /** How often asked actions prompt: false asks every time, true remembers one approval. */
  readonly remember: boolean;
  /**
   * Agent keys this protection is limited to. Empty means every key and caller.
   * A limited protection always asks every time: a remembered approval is
   * recorded for the account, so other keys could reuse it.
   */
  readonly keyIds: readonly string[];
  /** Keyed by `ProtectionAction.id`; absent means "follow the base". */
  readonly overrides: Readonly<Record<string, ActionSetting>>;
  /** Rules authored elsewhere that this editor does not understand. */
  readonly preserved: readonly ApprovalRule[];
}

/** Mirrors the backend limit in `approval_policy.rs`. */
export const MAX_PROTECTION_RULES = 50;

const READ_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);

export function actionVerb(method: string): ApprovalVerb {
  const upper = method.trim().toUpperCase();
  if (READ_METHODS.has(upper)) return "read";
  return upper === "DELETE" ? "destructive" : "write";
}

/**
 * Turns an OpenAPI-style path into the glob the approval engine matches.
 * Path parameters become one-segment wildcards; glob metacharacters in
 * literal segments are escaped so they cannot widen the match.
 */
export function actionPattern(path: string): string {
  const withSlash = path.trim().startsWith("/") ? path.trim() : `/${path.trim()}`;
  const withoutQuery = withSlash.split("?")[0] ?? withSlash;
  return withoutQuery
    .split("/")
    .map((segment) =>
      /^\{[^{}]*\}$/.test(segment) || /^:[\w-]+$/.test(segment)
        ? "*"
        : segment.replace(/[\\*?[\]{}!]/g, (char) => `\\${char}`),
    )
    .join("/");
}

export function baseSetting(
  draft: Pick<ProtectionDraft, "base" | "remember">,
  verb: ApprovalVerb,
): ActionSetting {
  const asks = draft.base === "ask_all" || (draft.base === "ask_changes" && verb !== "read");
  return { choice: asks ? "ask" : "allow", everyTime: !draft.remember };
}

export function effectiveSetting(
  draft: ProtectionDraft,
  action: ProtectionAction,
): ActionSetting {
  return draft.overrides[action.id] ?? baseSetting(draft, actionVerb(action.method));
}

function sameSetting(a: ActionSetting, b: ActionSetting): boolean {
  return a.choice === b.choice && (a.choice !== "ask" || a.everyTime === b.everyTime);
}

/** Drops an override that only restates the base, so rules stay minimal. */
export function withActionSetting(
  draft: ProtectionDraft,
  action: ProtectionAction,
  setting: ActionSetting,
): ProtectionDraft {
  const overrides = { ...draft.overrides };
  if (setting.choice !== "ask" && sameSetting(setting, baseSetting(draft, actionVerb(action.method)))) {
    delete overrides[action.id];
  } else {
    overrides[action.id] = setting;
  }
  return { ...draft, overrides };
}

function sameKeys(a: readonly string[] | undefined, b: readonly string[]): boolean {
  const left = a ?? [];
  return left.length === b.length && left.every((id) => b.includes(id));
}

function withRuleKeys(rule: ApprovalRule, keyIds: readonly string[]): ApprovalRule {
  const rest: ApprovalRule = {
    methods: rule.methods,
    resource_pattern: rule.resource_pattern,
    verbs: rule.verbs,
    effect: rule.effect,
    mode: rule.mode,
  };
  return keyIds.length ? { ...rest, mode: "per_request", api_key_ids: [...keyIds] } : rest;
}

/** Limits the protection to these agent keys, or to every caller when empty. */
export function withKeys(
  draft: ProtectionDraft,
  keyIds: readonly string[],
  actions: readonly ProtectionAction[],
): ProtectionDraft {
  const limited = keyIds.length > 0;
  let next: ProtectionDraft = {
    ...draft,
    keyIds: [...keyIds],
    remember: limited ? false : draft.remember,
    overrides: {},
    preserved: draft.preserved.map((rule) => withRuleKeys(rule, keyIds)),
  };
  for (const action of actions) {
    const existing = draft.overrides[action.id];
    if (existing) {
      next = withActionSetting(next, action, {
        ...existing,
        everyTime: limited ? true : existing.everyTime,
      });
    }
  }
  return next;
}

/** Changes the starting point or how often it asks, keeping individual choices. */
export function withBase(
  draft: ProtectionDraft,
  change: Partial<Pick<ProtectionDraft, "base" | "remember">>,
  actions: readonly ProtectionAction[],
): ProtectionDraft {
  let next: ProtectionDraft = {
    ...draft,
    ...change,
    remember: draft.keyIds.length ? false : (change.remember ?? draft.remember),
    overrides: {},
  };
  for (const action of actions) {
    const existing = draft.overrides[action.id];
    if (existing) next = withActionSetting(next, action, existing);
  }
  return next;
}

function actionRule(action: ProtectionAction, setting: ActionSetting): ApprovalRule {
  return {
    methods: [action.method.trim().toUpperCase()],
    resource_pattern: actionPattern(action.path),
    verbs: [],
    effect:
      setting.choice === "allow"
        ? "auto_allow"
        : setting.choice === "block"
          ? "deny"
          : "require_approval",
    mode: setting.choice === "ask" && !setting.everyTime ? "grant" : "per_request",
  };
}

function matchesEverything(rule: ApprovalRule): boolean {
  return (
    rule.methods.length === 0 &&
    (rule.resource_pattern.trim() === "" || rule.resource_pattern.trim() === "*")
  );
}

function isEverythingRule(rule: ApprovalRule): boolean {
  return rule.effect === "require_approval" && matchesEverything(rule) && rule.verbs.length === 0;
}

/** Ends a key's own policy so its calls never fall through to the All keys rules. */
function isAllowRestRule(rule: ApprovalRule): boolean {
  return rule.effect === "auto_allow" && matchesEverything(rule) && rule.verbs.length === 0;
}

function isChangesRule(rule: ApprovalRule): boolean {
  return (
    rule.effect === "require_approval" &&
    matchesEverything(rule) &&
    rule.verbs.length === 2 &&
    rule.verbs.includes("write") &&
    rule.verbs.includes("destructive")
  );
}

const CHANGES_RULE: ApprovalRule = {
  methods: [],
  resource_pattern: "*",
  verbs: ["write", "destructive"],
  effect: "require_approval",
  mode: "per_request",
};

/**
 * Everything stored for one service: an optional protection for every caller
 * and any number of protections limited to specific agent keys. A key with
 * its own protection follows only that one.
 */
export interface ServicePolicy {
  readonly all: ProtectionDraft | null;
  readonly keyed: readonly ProtectionDraft[];
}

export interface ProtectionPayload {
  readonly approvalRequired: boolean;
  readonly approvalMode: ApprovalMode;
  /** Absent when no All keys protection exists, so other callers are untouched. */
  readonly defaultEffect?: ApprovalEffect;
  readonly rules: readonly ApprovalRule[];
}

type StoredPolicy = Pick<
  ServiceApprovalConfigItem,
  "rules" | "default_effect" | "approval_required" | "approval_mode"
>;

export function newProtection(keyIds: readonly string[] = []): ProtectionDraft {
  return { base: "ask_changes", remember: false, keyIds: [...keyIds], overrides: {}, preserved: [] };
}

function readOverrides(
  draft: ProtectionDraft,
  rules: readonly ApprovalRule[],
  actions: readonly ProtectionAction[],
  isBaseRule: (rule: ApprovalRule) => boolean,
): ProtectionDraft {
  const byKey = new Map(
    actions.map((action) => [
      `${action.method.trim().toUpperCase()} ${actionPattern(action.path)}`,
      action,
    ]),
  );
  let next = draft;
  const preserved: ApprovalRule[] = [];
  for (const rule of rules) {
    if (isBaseRule(rule)) continue;
    const action =
      rule.methods.length === 1 && rule.verbs.length === 0
        ? byKey.get(`${rule.methods[0]!.toUpperCase()} ${rule.resource_pattern.trim()}`)
        : undefined;
    if (!action || next.overrides[action.id]) {
      preserved.push(rule);
      continue;
    }
    next = {
      ...next,
      overrides: {
        ...next.overrides,
        [action.id]: {
          choice:
            rule.effect === "auto_allow" ? "allow" : rule.effect === "deny" ? "block" : "ask",
          everyTime: rule.mode !== "grant",
        },
      },
    };
  }
  return { ...next, preserved };
}

function readKeyed(
  keyIds: readonly string[],
  rules: readonly ApprovalRule[],
  actions: readonly ProtectionAction[],
): ProtectionDraft {
  const base: BaseChoice = rules.some(isEverythingRule)
    ? "ask_all"
    : rules.some(isChangesRule)
      ? "ask_changes"
      : "allow_all";
  return readOverrides(
    { base, remember: false, keyIds: [...keyIds], overrides: {}, preserved: [] },
    rules,
    actions,
    (rule) => isEverythingRule(rule) || isChangesRule(rule) || isAllowRestRule(rule),
  );
}

function readAll(
  config: StoredPolicy,
  rules: readonly ApprovalRule[],
  actions: readonly ProtectionAction[],
): ProtectionDraft {
  const defaultAsks =
    config.default_effect === "require_approval" ||
    (config.default_effect === null && config.rules.length === 0 && config.approval_required);
  const changesRule = rules.find(isChangesRule);
  const base: BaseChoice = defaultAsks ? "ask_all" : changesRule ? "ask_changes" : "allow_all";
  const remember =
    base === "ask_all"
      ? config.approval_mode === "grant"
      : base === "ask_changes" && changesRule?.mode === "grant";
  return readOverrides(
    { base, remember, keyIds: [], overrides: {}, preserved: [] },
    rules,
    actions,
    isChangesRule,
  );
}

/** Reads a stored policy back into the editor's shape. */
export function parsePolicy(
  config: StoredPolicy | null,
  actions: readonly ProtectionAction[],
): ServicePolicy {
  if (!config) return { all: null, keyed: [] };
  const groups: { keyIds: readonly string[]; rules: ApprovalRule[] }[] = [];
  const general: ApprovalRule[] = [];
  for (const rule of config.rules) {
    const keyIds = rule.api_key_ids ?? [];
    if (!keyIds.length) {
      general.push(rule);
      continue;
    }
    const group = groups.find((entry) => sameKeys(entry.keyIds, keyIds));
    if (group) group.rules.push(rule);
    else groups.push({ keyIds, rules: [rule] });
  }
  const hasGeneral =
    general.length > 0 || config.default_effect !== null || config.rules.length === 0;
  return {
    all: hasGeneral ? readAll(config, general, actions) : null,
    keyed: groups.map((group) => readKeyed(group.keyIds, group.rules, actions)),
  };
}

function overrideRules(
  draft: ProtectionDraft,
  actions: readonly ProtectionAction[],
): ApprovalRule[] {
  const rules: ApprovalRule[] = [...draft.preserved];
  const ordered = [...actions].sort((a, b) => {
    const left = actionPattern(a.path), right = actionPattern(b.path);
    return (left.match(/(?<!\\)\*/g)?.length ?? 0) - (right.match(/(?<!\\)\*/g)?.length ?? 0) || right.length - left.length;
  });
  for (const action of ordered) {
    const setting = draft.overrides[action.id];
    if (setting) rules.push(actionRule(action, setting));
  }
  return rules;
}

/**
 * Key protections come first and each ends with a catch-all, so a key with
 * its own protection never reaches the All keys rules or default.
 */
export function buildPolicy(
  policy: ServicePolicy,
  actions: readonly ProtectionAction[],
): ProtectionPayload {
  const rules: ApprovalRule[] = [];
  for (const draft of policy.keyed) {
    const own = overrideRules(draft, actions);
    if (draft.base === "ask_changes") own.push(CHANGES_RULE);
    own.push({
      ...CHANGES_RULE,
      verbs: [],
      effect: draft.base === "ask_all" ? "require_approval" : "auto_allow",
    });
    rules.push(...own.map((rule) => withRuleKeys(rule, draft.keyIds)));
  }
  const all = policy.all;
  if (!all) return { approvalMode: "per_request", approvalRequired: true, rules };
  const approvalMode: ApprovalMode = all.remember ? "grant" : "per_request";
  rules.push(...overrideRules(all, actions));
  if (all.base === "ask_changes") rules.push({ ...CHANGES_RULE, mode: approvalMode });
  return {
    approvalMode,
    approvalRequired:
      all.base !== "allow_all" || rules.some((rule) => rule.effect !== "auto_allow"),
    defaultEffect: all.base === "ask_all" ? "require_approval" : "auto_allow",
    rules,
  };
}

/** Whether two protections cover the same callers. */
export function sameScope(a: readonly string[], b: readonly string[]): boolean {
  return sameKeys(a, b);
}

/** Adds the protection, replacing the one that covers the same callers. */
export function withProtection(policy: ServicePolicy, draft: ProtectionDraft): ServicePolicy {
  if (!draft.keyIds.length) return { ...policy, all: draft };
  const index = policy.keyed.findIndex((entry) => sameKeys(entry.keyIds, draft.keyIds));
  return {
    ...policy,
    keyed:
      index < 0
        ? [...policy.keyed, draft]
        : policy.keyed.map((entry, position) => (position === index ? draft : entry)),
  };
}

export function withoutProtection(
  policy: ServicePolicy,
  keyIds: readonly string[],
): ServicePolicy {
  if (!keyIds.length) return { ...policy, all: null };
  return { ...policy, keyed: policy.keyed.filter((entry) => !sameKeys(entry.keyIds, keyIds)) };
}

export function isEmptyPolicy(policy: ServicePolicy): boolean {
  return policy.all === null && policy.keyed.length === 0;
}

export interface ProtectionSummary {
  readonly ask: number;
  readonly block: number;
  readonly allow: number;
}

/**
 * Counts individually set actions. Parse with an empty action list for list
 * rows: every per-action rule then lands in `preserved` and is counted there.
 */
export function summarizeProtection(draft: ProtectionDraft): ProtectionSummary {
  const settings = [
    ...draft.preserved.map((rule) => rule.effect),
    ...Object.values(draft.overrides).map((setting) =>
      setting.choice === "ask"
        ? "require_approval"
        : setting.choice === "block"
          ? "deny"
          : "auto_allow",
    ),
  ];
  return {
    ask: settings.filter((effect) => effect === "require_approval").length,
    block: settings.filter((effect) => effect === "deny").length,
    allow: settings.filter((effect) => effect === "auto_allow").length,
  };
}

/** A guided edit is permitted only for policies this compiler can reproduce
 * exactly, with no carried-over advanced rules or inferred fallback changes. */
export function isWizardPolicy(config: ServiceApprovalConfigItem, actions: readonly ProtectionAction[]): boolean {
  if (config.disabled) return false;
  const parsed = parsePolicy(config, actions);
  if ([...(parsed.all ? [parsed.all] : []), ...parsed.keyed].some((draft) => draft.preserved.length > 0)) return false;
  const built = buildPolicy(parsed, actions);
  const rule = (item: ApprovalRule) => ({ methods: item.methods, resource_pattern: item.resource_pattern, verbs: item.verbs, effect: item.effect, mode: item.mode, api_key_ids: item.api_key_ids ?? [], mandatory_review: item.mandatory_review ?? false });
  return config.approval_required === built.approvalRequired && config.approval_mode === built.approvalMode && config.default_effect === (built.defaultEffect ?? null) && JSON.stringify(config.rules.map(rule)) === JSON.stringify(built.rules.map(rule));
}
