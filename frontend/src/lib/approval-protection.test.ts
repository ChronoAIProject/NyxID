import { describe, expect, it } from "vitest";
import {
  actionPattern,
  actionVerb,
  buildPolicy,
  effectiveSetting,
  isEmptyPolicy,
  isWizardPolicy,
  newProtection,
  parsePolicy,
  summarizeProtection,
  withActionSetting,
  withBase,
  withKeys,
  withProtection,
  withoutProtection,
  type ProtectionAction,
  type ProtectionPayload,
} from "./approval-protection";

const merge: ProtectionAction = {
  id: "merge",
  name: "Merge pull request",
  method: "put",
  path: "/repos/{owner}/{repo}/pulls/{number}/merge",
};
const list: ProtectionAction = { id: "list", name: "List repositories", method: "GET", path: "user/repos" };
const remove: ProtectionAction = { id: "remove", name: "Delete repository", method: "DELETE", path: "/repos/{owner}/{repo}" };
const actions = [merge, list, remove];

const stored = (payload: ProtectionPayload) => ({
  rules: payload.rules,
  default_effect: payload.defaultEffect ?? null,
  approval_required: payload.approvalRequired,
  approval_mode: payload.approvalMode,
});
const only = (draft: ReturnType<typeof newProtection>) => withProtection({ all: null, keyed: [] }, draft);

describe("approval protection", () => {
  it("classifies methods the way the approval engine does", () => {
    expect(actionVerb("get")).toBe("read");
    expect(actionVerb("POST")).toBe("write");
    expect(actionVerb("delete")).toBe("destructive");
  });

  it("turns path parameters into one-segment wildcards and escapes glob characters", () => {
    expect(actionPattern(merge.path)).toBe("/repos/*/*/pulls/*/merge");
    expect(actionPattern("user/repos?page=1")).toBe("/user/repos");
    expect(actionPattern("/files/a*b/{id}")).toBe("/files/a\\*b/*");
    expect(actionPattern("/v1/{a,b}x")).toBe("/v1/\\{a,b\\}x");
  });

  it("emits only overrides plus the changes rule for an All keys protection", () => {
    let draft = newProtection();
    expect(effectiveSetting(draft, list).choice).toBe("allow");
    expect(effectiveSetting(draft, merge)).toEqual({ choice: "ask", everyTime: true });
    draft = withActionSetting(draft, remove, { choice: "block", everyTime: true });
    draft = withActionSetting(draft, merge, { choice: "ask", everyTime: true });
    expect(buildPolicy(only(draft), actions)).toEqual({
      approvalRequired: true,
      approvalMode: "per_request",
      defaultEffect: "auto_allow",
      rules: [
        { methods: ["DELETE"], resource_pattern: "/repos/*/*", verbs: [], effect: "deny", mode: "per_request" },
        { methods: ["PUT"], resource_pattern: "/repos/*/*/pulls/*/merge", verbs: [], effect: "require_approval", mode: "per_request" },
        { methods: [], resource_pattern: "*", verbs: ["write", "destructive"], effect: "require_approval", mode: "per_request" },
      ],
    });
  });

  it("round-trips an All keys protection, including remembered approvals", () => {
    let draft = withBase(newProtection(), { base: "allow_all" }, actions);
    draft = withActionSetting(draft, merge, { choice: "ask", everyTime: false });
    draft = withActionSetting(draft, list, { choice: "allow", everyTime: true });
    const payload = buildPolicy(only(draft), actions);
    expect(payload.rules).toEqual([
      { methods: ["PUT"], resource_pattern: "/repos/*/*/pulls/*/merge", verbs: [], effect: "require_approval", mode: "grant" },
    ]);
    expect(parsePolicy(stored(payload), actions)).toEqual({ all: draft, keyed: [] });

    const remembered = withBase(newProtection(), { remember: true }, actions);
    const rememberedPayload = buildPolicy(only(remembered), actions);
    expect(rememberedPayload.approvalMode).toBe("grant");
    expect(parsePolicy(stored(rememberedPayload), actions).all).toEqual(remembered);
    const everything = buildPolicy(only(withBase(remembered, { base: "ask_all" }, actions)), actions);
    expect(parsePolicy(stored(everything), actions).all?.remember).toBe(true);
  });

  it("keeps rules it does not understand ahead of its own", () => {
    const foreign = { methods: ["POST"], resource_pattern: "/admin/**", verbs: [], effect: "deny", mode: "per_request" } as const;
    const policy = parsePolicy(
      { rules: [foreign], default_effect: "require_approval", approval_required: true, approval_mode: "per_request" },
      actions,
    );
    expect(policy.all?.base).toBe("ask_all");
    expect(policy.all?.preserved).toEqual([foreign]);
    expect(buildPolicy(policy, actions).rules[0]).toEqual(foreign);
  });

  it("reads legacy on/off policies", () => {
    const legacy = (approval_required: boolean) =>
      parsePolicy({ rules: [], default_effect: null, approval_required, approval_mode: "per_request" }, actions).all?.base;
    expect(legacy(true)).toBe("ask_all");
    expect(legacy(false)).toBe("allow_all");
  });

  it("gives a key its own complete protection that never falls through", () => {
    let keyed = withKeys(withBase(newProtection(), { remember: true }, actions), ["key-a"], actions);
    expect(keyed.remember).toBe(false);
    keyed = withActionSetting(keyed, remove, { choice: "block", everyTime: true });
    const payload = buildPolicy(only(keyed), actions);
    expect(payload.defaultEffect).toBeUndefined();
    expect(payload.rules).toEqual([
      { methods: ["DELETE"], resource_pattern: "/repos/*/*", verbs: [], effect: "deny", mode: "per_request", api_key_ids: ["key-a"] },
      { methods: [], resource_pattern: "*", verbs: ["write", "destructive"], effect: "require_approval", mode: "per_request", api_key_ids: ["key-a"] },
      // Catch-all: this key's reads are allowed here and never reach other rules.
      { methods: [], resource_pattern: "*", verbs: [], effect: "auto_allow", mode: "per_request", api_key_ids: ["key-a"] },
    ]);
    expect(parsePolicy(stored(payload), actions)).toEqual({ all: null, keyed: [keyed] });

    const everything = buildPolicy(only(withBase(keyed, { base: "ask_all" }, actions)), actions);
    expect(everything.rules.at(-1)).toMatchObject({ verbs: [], effect: "require_approval", api_key_ids: ["key-a"] });
    expect(parsePolicy(stored(everything), actions).keyed[0]?.base).toBe("ask_all");
  });

  it("holds an All keys protection and separate key protections side by side", () => {
    const general = withBase(newProtection(), { base: "ask_all", remember: true }, actions);
    const forA = withBase(newProtection(["key-a"]), { base: "allow_all" }, actions);
    const forB = withActionSetting(newProtection(["key-b"]), list, { choice: "block", everyTime: true });
    let policy = withProtection(withProtection(only(general), forA), forB);
    const payload = buildPolicy(policy, actions);
    // Key rules first, each closed by its catch-all, then the general rules and default.
    expect(payload.rules.map((rule) => [rule.api_key_ids?.join() ?? "all", rule.effect])).toEqual([
      ["key-a", "auto_allow"],
      ["key-b", "deny"],
      ["key-b", "require_approval"],
      ["key-b", "auto_allow"],
    ]);
    expect(payload.defaultEffect).toBe("require_approval");
    expect(payload.approvalMode).toBe("grant");
    expect(parsePolicy(stored(payload), actions)).toEqual(policy);

    // Editing one key leaves the others and the general protection as they were.
    policy = withProtection(policy, withBase(forA, { base: "ask_all" }, actions));
    expect(policy.keyed.map((draft) => draft.base)).toEqual(["ask_all", "ask_changes"]);
    expect(policy.all).toEqual(general);

    policy = withoutProtection(policy, []);
    expect(buildPolicy(policy, actions).defaultEffect).toBeUndefined();
    policy = withoutProtection(withoutProtection(policy, ["key-a"]), ["key-b"]);
    expect(isEmptyPolicy(policy)).toBe(true);
  });

  it("summarises individually set actions with or without the action list", () => {
    const draft = withActionSetting(newProtection(), remove, { choice: "block", everyTime: true });
    expect(summarizeProtection(draft)).toEqual({ ask: 0, block: 1, allow: 0 });
    const reread = parsePolicy(stored(buildPolicy(only(draft), actions)), []).all!;
    expect(summarizeProtection(reread)).toEqual({ ask: 0, block: 1, allow: 0 });
  });
});

it("keeps a literal Block ahead of an overlapping parameterized Allow",()=>{
  const broad:ProtectionAction={id:"broad",name:"Any",method:"GET",path:"/items/{id}"};
  const literal:ProtectionAction={id:"literal",name:"Status",method:"GET",path:"/items/status"};
  let draft=withBase(newProtection(),{base:"ask_all"},[broad,literal]);
  draft=withActionSetting(draft,broad,{choice:"allow",everyTime:true});
  draft=withActionSetting(draft,literal,{choice:"block",everyTime:true});
  expect(buildPolicy(only(draft),[broad,literal]).rules.map((rule)=>[rule.resource_pattern,rule.effect])).toEqual([["/items/status","deny"],["/items/*","auto_allow"]]);
});
it("retains an explicit POST Ask even when method heuristics agree with the base",()=>{
  const post:ProtectionAction={id:"query",name:"Query",method:"POST",path:"/query"};
  const draft=withActionSetting(newProtection(),post,{choice:"ask",everyTime:true});
  expect(buildPolicy(only(draft),[post]).rules[0]).toMatchObject({methods:["POST"],resource_pattern:"/query",effect:"require_approval"});
});
it("only permits guided editing when the complete policy round-trips exactly",()=>{
  const authored=buildPolicy(only(newProtection()),[]);
  const config={...stored(authored),service_id:"svc",service_name:"Service",created_at:"old",updated_at:"old"};
  expect(isWizardPolicy(config,[])).toBe(true);
  expect(isWizardPolicy({...config,default_effect:"deny"},[])).toBe(false);
  expect(isWizardPolicy({...config,default_effect:null,rules:[]},[])).toBe(false);
  const foreign=[{methods:["EXEC"],resource_pattern:"git *",verbs:[],effect:"require_approval",mode:"grant"}] as const;
  expect(isWizardPolicy({...config,rules:foreign},[])).toBe(false);
});
