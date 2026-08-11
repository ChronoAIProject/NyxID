import { useReducer } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  directAssistantTransport,
  type DirectConversationSettings,
} from "@/lib/assistant/direct-transport";
import {
  directModelsSchema,
  directSkillsSchema,
} from "@/schemas/assistant-direct";

const directCatalogKeys = {
  skills: ["assistant", "direct", "skills"] as const,
  models: ["assistant", "direct", "models"] as const,
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

export function useDirectConversationSettings(
  conversationId: string | undefined,
  defaultModel: string | undefined,
): {
  readonly settings: DirectConversationSettings;
  readonly setModel: (model: string) => void;
  readonly setSkill: (skillSlug: string | null) => void;
} {
  const [, forceRender] = useReducer((revision: number) => revision + 1, 0);

  if (defaultModel) {
    directAssistantTransport.seedDefaultModel(conversationId, defaultModel);
  }
  const settings = directAssistantTransport.getSettings(conversationId);

  return {
    settings,
    setModel: (model) => {
      directAssistantTransport.setModel(conversationId, model);
      forceRender();
    },
    setSkill: (skillSlug) => {
      directAssistantTransport.setSkill(conversationId, skillSlug);
      forceRender();
    },
  };
}
