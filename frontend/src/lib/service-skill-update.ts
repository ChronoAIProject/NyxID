import type { DownstreamService } from "@/types/api";

export function serviceSkillUpdate(
  observed: Pick<
    DownstreamService,
    "recommended_skills" | "recommended_skill_refs" | "skills_revision"
  >,
  input: string | undefined,
  clearRefs: boolean | undefined,
) {
  const names = (input ?? "")
    .split(/[,\n]/)
    .map((name) => name.trim())
    .filter(Boolean);
  const changed =
    JSON.stringify(names) !== JSON.stringify(observed.recommended_skills ?? []);
  if (!changed && !clearRefs) return {};
  if (changed && observed.recommended_skill_refs != null && !clearRefs) {
    throw new Error(
      "Clear the pinned references explicitly before changing these skill names.",
    );
  }
  return {
    recommended_skills: names,
    skills_revision: observed.skills_revision ?? 0,
    ...(clearRefs ? { clear_skill_refs: true } : {}),
  };
}

export interface SkillRequestIdentity {
  fingerprint: string;
  id: string;
}

export function skillRequestIdentity(
  payload: unknown,
  previous?: SkillRequestIdentity,
) {
  const fingerprint = JSON.stringify(payload);
  return previous?.fingerprint === fingerprint
    ? previous
    : { fingerprint, id: crypto.randomUUID() };
}
