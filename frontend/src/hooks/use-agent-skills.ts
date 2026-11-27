import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import {
  agentSkillsSchema,
  skillSearchSchema,
  skillVersionsSchema,
  skillPreviewSchema,
  type AgentSkill,
  type SkillPin,
  attachedSkillPreviewSchema,
  agentSkillHistorySchema,
  skillImproveSchema,
} from "@/schemas/agent-skills";
const base = "/assistant/nyxagent";

function agentSkillsPath(id: string) {
  return `${base}/agents/${encodeURIComponent(id)}/skills`;
}

async function readAgentSkills(id: string, authorityGuard?: () => void) {
  return agentSkillsSchema.parse(
    await assistantJson(agentSkillsPath(id), { authorityGuard }),
  );
}

function writeAgentSkills(
  id: string,
  selection: { expected_revision: number; skills: AgentSkill[] },
  authorityGuard?: () => void,
) {
  return assistantJson(agentSkillsPath(id), { method: "PUT", body: selection, authorityGuard });
}

function sameSkillPin(left: SkillPin, right: SkillPin) {
  return left.source === right.source && left.skill_id === right.skill_id &&
    left.version === right.version && left.sha256 === right.sha256;
}

export function useAgentSkills(id: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["agent-skills", owner, id],
    queryFn: () => readAgentSkills(id),
    enabled: Boolean(owner && id),
    retry: false,
  });
}
export function useSetAgentSkills(id: string) {
  const client = useQueryClient();
  const owner = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: (selection: { expected_revision: number; skills: AgentSkill[] }) =>
      writeAgentSkills(id, selection),
    onSuccess: () =>
      Promise.all([
        client.invalidateQueries({ queryKey: ["agent-skills", owner, id] }),
        client.invalidateQueries({
          queryKey: ["agent-skill-history", owner, id],
        }),
      ]),
  });
}

/** An explicit template confirmation may append a pin or reuse an exact enabled one. */
export function useAttachTemplateSkill() {
  const client = useQueryClient();
  const owner = useAuthStore((state) => state.user?.id);
  return async (
    agentId: string,
    reference: AgentSkill,
    authorityGuard: () => void,
  ): Promise<"ready" | "disabled" | "different" | "full"> => {
    const current = await readAgentSkills(agentId, authorityGuard);
    const existing = current.skills.find((skill) => skill.skill_id === reference.skill_id);
    if (existing) {
      const sameDependencies = existing.dependencies.length === reference.dependencies.length &&
        existing.dependencies.every((pin) => reference.dependencies.some((entry) => sameSkillPin(pin, entry)));
      if (!sameSkillPin(existing, reference) || !sameDependencies) return "different";
      return existing.disabled ? "disabled" : "ready";
    }
    if (current.skills.length >= 16) return "full";
    await writeAgentSkills(agentId, {
      expected_revision: current.revision,
      skills: [...current.skills, reference],
    }, authorityGuard);
    void Promise.all([
      client.invalidateQueries({ queryKey: ["agent-skills", owner, agentId] }),
      client.invalidateQueries({ queryKey: ["agent-skill-history", owner, agentId] }),
    ]);
    return "ready";
  };
}
export function useSetSkillPreload(agentId: string, skillId: string) {
  const client = useQueryClient();
  const owner = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: (selection: { expected_revision: number; paths: string[] }) =>
      assistantJson(
        `${base}/agents/${encodeURIComponent(agentId)}/skills/${encodeURIComponent(skillId)}/preload`,
        {
          method: "PUT",
          body: selection,
        },
      ),
    onSuccess: () =>
      Promise.all([
        client.invalidateQueries({
          queryKey: ["agent-skills", owner, agentId],
        }),
        client.invalidateQueries({
          queryKey: ["agent-skill-history", owner, agentId],
        }),
      ]),
  });
}
export function useSkillSearch(query: string | null, page: number) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["ornn-skills", owner, query, page],
    queryFn: async () =>
      skillSearchSchema.parse(
        await assistantJson(
          `${base}/skills/catalog?q=${encodeURIComponent(query ?? "")}&page=${page}`,
        ),
      ),
    enabled: Boolean(owner) && query !== null,
    retry: false,
  });
}
function skillVersionsQuery(
  owner: string | undefined,
  id: string | undefined,
  page: number,
) {
  return {
    queryKey: ["ornn-versions", owner, id, page],
    queryFn: async () =>
      skillVersionsSchema.parse(
        await assistantJson(
          `${base}/skills/catalog?skill=${encodeURIComponent(id!)}&page=${page}`,
        ),
      ),
    retry: false,
  } as const;
}
export async function readSkillPreview(id: string, version: string, authorityGuard?: () => void) {
  return skillPreviewSchema.parse(await assistantJson(
    `${base}/skills/catalog?skill=${encodeURIComponent(id)}&version=${encodeURIComponent(version)}`,
    { authorityGuard },
  ));
}

function skillPreviewQuery(
  owner: string | undefined,
  id: string | undefined,
  version: string,
) {
  return {
    queryKey: ["ornn-preview", owner, id, version],
    queryFn: () => readSkillPreview(id!, version),
    retry: false,
  } as const;
}
export function useSkillVersions(id: string | undefined, page = 1) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    ...skillVersionsQuery(owner, id, page),
    enabled: Boolean(owner && id),
  });
}
export function useSkillPreview(id: string | undefined, version: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    ...skillPreviewQuery(owner, id, version),
    enabled: Boolean(owner && id && version),
  });
}

/**
 * Attaches a catalog skill at its newest non-deprecated version, pinned
 * exactly: the version and sha256 come from the verified package preview,
 * never a floating "latest".
 */
export function useAttachLatestSkill(agentId: string) {
  const client = useQueryClient();
  const owner = useAuthStore((s) => s.user?.id);
  const write = useSetAgentSkills(agentId);
  return useMutation({
    mutationFn: async ({
      skillId,
      current,
    }: {
      skillId: string;
      current: { readonly revision: number; readonly skills: AgentSkill[] };
    }) => {
      const versions = await client.fetchQuery(
        skillVersionsQuery(owner, skillId, 1),
      );
      const version = versions.items.find((item) => !item.deprecated);
      if (!version) {
        throw new Error(
          "This skill has no supported version to attach. Choose another skill.",
        );
      }
      const preview = await client.fetchQuery(
        skillPreviewQuery(owner, skillId, version.version),
      );
      if (preview.reference.sha256 !== version.sha256) {
        throw new Error(
          "Ornn returned different content for this version. Try again.",
        );
      }
      return write.mutateAsync({
        expected_revision: current.revision,
        skills: [
          ...current.skills.filter((pin) => pin.skill_id !== skillId),
          preview.reference,
        ],
      });
    },
  });
}
/** Explicit per-pin test: reads the pinned version the way the agent does. */
export function useAttachedSkillPreview(agentId: string, skillId: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["attached-skill-preview", owner, agentId, skillId],
    queryFn: async () =>
      attachedSkillPreviewSchema.parse(
        await assistantJson(
          `${base}/agents/${encodeURIComponent(agentId)}/skills/${encodeURIComponent(skillId)}/preview`,
        ),
      ),
    enabled: Boolean(owner && agentId && skillId),
    retry: false,
  });
}
export function useAgentSkillHistory(agentId: string) {
  const owner = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["agent-skill-history", owner, agentId],
    queryFn: async () =>
      agentSkillHistorySchema.parse(
        await assistantJson(
          `${base}/agents/${encodeURIComponent(agentId)}/skills/history`,
        ),
      ),
    enabled: Boolean(owner && agentId),
    retry: false,
  });
}

/**
 * Starts an authored improve draft of an attached skill. The server raises
 * its owner review card; the draft then appears in the learning proposals.
 */
export function useImproveAgentSkill(agentId: string) {
  const client = useQueryClient();
  const owner = useAuthStore((s) => s.user?.id);
  return useMutation({
    mutationFn: async (skillId: string) =>
      skillImproveSchema.parse(
        await assistantJson(
          `${base}/agents/${encodeURIComponent(agentId)}/skills/${encodeURIComponent(skillId)}/improve`,
          { method: "POST", body: {} },
        ),
      ),
    onSuccess: () =>
      client.invalidateQueries({
        queryKey: ["agent-learning-proposals", owner, agentId],
      }),
  });
}
