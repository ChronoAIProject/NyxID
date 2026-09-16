import { describe, expect, it } from "vitest";
import {
  serviceSkillUpdate,
  skillRequestIdentity,
} from "./service-skill-update";

describe("service skill edits", () => {
  it("omits unchanged names including legacy empty metadata", () => {
    expect(
      serviceSkillUpdate(
        { recommended_skills: ["ornn/manual"], skills_revision: 4 },
        " ornn/manual ",
        false,
      ),
    ).toEqual({});
    expect(serviceSkillUpdate({}, "", false)).toEqual({});
  });

  it("uses the observed revision for assignment and clearing", () => {
    expect(
      serviceSkillUpdate(
        { recommended_skills: ["old"], skills_revision: 7 },
        "new, another",
        false,
      ),
    ).toEqual({ recommended_skills: ["new", "another"], skills_revision: 7 });
    expect(
      serviceSkillUpdate({ recommended_skills: ["old"] }, "", false),
    ).toEqual({ recommended_skills: [], skills_revision: 0 });
  });

  it("requires explicit clearing of existing immutable refs", () => {
    const observed = {
      recommended_skills: ["old"],
      skills_revision: 3,
      recommended_skill_refs: [
        {
          source: "ornn",
          skill_id: "skill",
          name: "old",
          version: "1.0",
          sha256: "a".repeat(64),
          dependencies: [],
        },
      ],
    };
    expect(() => serviceSkillUpdate(observed, "new", false)).toThrow(
      "explicitly",
    );
    expect(serviceSkillUpdate(observed, "new", true)).toEqual({
      recommended_skills: ["new"],
      skills_revision: 3,
      clear_skill_refs: true,
    });
    expect(serviceSkillUpdate(observed, "old", true)).toEqual({
      recommended_skills: ["old"],
      skills_revision: 3,
      clear_skill_refs: true,
    });
  });

  it("requires and permits clearing an empty pinned list", () => {
    const observed = {
      recommended_skills: [],
      recommended_skill_refs: [],
      skills_revision: 2,
    };
    expect(() => serviceSkillUpdate(observed, "new", false)).toThrow(
      "explicitly",
    );
    expect(serviceSkillUpdate(observed, "new", true)).toEqual({
      recommended_skills: ["new"],
      skills_revision: 2,
      clear_skill_refs: true,
    });
  });

  it("binds the operation ID to the target service", () => {
    const payload = { recommended_skills: ["one"], skills_revision: 0 };
    const first = skillRequestIdentity({ serviceId: "first", payload });
    expect(
      skillRequestIdentity({ serviceId: "second", payload }, first).id,
    ).not.toBe(first.id);
  });

  it("reuses an operation ID only for identical complete payloads", () => {
    const payload = {
      name: "First",
      recommended_skills: ["one"],
      skills_revision: 2,
    };
    const first = skillRequestIdentity(payload);
    expect(skillRequestIdentity({ ...payload }, first)).toBe(first);
    expect(
      skillRequestIdentity({ ...payload, name: "Second" }, first).id,
    ).not.toBe(first.id);
    expect(
      skillRequestIdentity({ ...payload, skills_revision: 3 }, first).id,
    ).not.toBe(first.id);
  });
});
