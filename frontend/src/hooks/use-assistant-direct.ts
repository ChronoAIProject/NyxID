import { useSyncExternalStore } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  directAssistantTransport,
  type DirectConversationSettings,
} from "@/lib/assistant/direct-transport";
import {
  directEffortsSchema,
  directModelsSchema,
  directSkillsSchema,
} from "@/schemas/assistant-direct";

const directCatalogKeys = {
  skills: ["assistant", "direct", "skills"] as const,
  models: ["assistant", "direct", "models"] as const,
  efforts: ["assistant", "direct", "efforts"] as const,
};

export function useDirectSkills() {
  return useQuery({
    queryKey: directCatalogKeys.skills,
    queryFn: async () =>
      directSkillsSchema.parse(
        await api.get<unknown>("/assistant/direct/skills"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectModels() {
  return useQuery({
    queryKey: directCatalogKeys.models,
    queryFn: async () =>
      directModelsSchema.parse(
        await api.get<unknown>("/assistant/direct/models"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectEfforts() {
  return useQuery({
    queryKey: directCatalogKeys.efforts,
    queryFn: async () =>
      directEffortsSchema.parse(
        await api.get<unknown>("/assistant/direct/efforts"),
      ),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useDirectConversationSettings(
  conversationId: string | undefined,
  defaultModel: string | undefined,
): {
  readonly settings: DirectConversationSettings;
  readonly canUpdate: boolean;
  readonly setModel: (model: string) => void;
  readonly setSkill: (skillSlug: string | null) => void;
  readonly setEffort: (effort: string | null) => void;
  readonly setAgentPocMode: (enabled: boolean) => void;
} {
  if (defaultModel) {
    directAssistantTransport.seedDefaultModel(conversationId, defaultModel);
  }
  const settings = useSyncExternalStore(
    directAssistantTransport.subscribeSettings,
    () => directAssistantTransport.getSettings(conversationId),
    () => directAssistantTransport.getSettings(conversationId),
  );

  return {
    settings,
    canUpdate: directAssistantTransport.canUpdateSettings(conversationId),
    setModel: (model) => {
      directAssistantTransport.setModel(conversationId, model);
    },
    setSkill: (skillSlug) => {
      directAssistantTransport.setSkill(conversationId, skillSlug);
    },
    setEffort: (effort) => {
      directAssistantTransport.setEffort(conversationId, effort);
    },
    setAgentPocMode: (enabled) => {
      directAssistantTransport.setAgentPocMode(conversationId, enabled);
    },
  };
}
