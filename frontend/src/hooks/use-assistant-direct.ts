import { useEffect, useState } from "react";
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
): {
  readonly settings: DirectConversationSettings;
  readonly setModel: (model: string) => void;
  readonly setSkill: (skillSlug: string | null) => void;
} {
  const [settings, setSettings] = useState(() =>
    directAssistantTransport.getSettings(conversationId),
  );

  useEffect(() => {
    setSettings(directAssistantTransport.getSettings(conversationId));
  }, [conversationId]);

  return {
    settings,
    setModel: (model) => {
      directAssistantTransport.setModel(conversationId, model);
      setSettings((current) => ({ ...current, model }));
    },
    setSkill: (skillSlug) => {
      directAssistantTransport.setSkill(conversationId, skillSlug);
      setSettings((current) => ({ ...current, skillSlug }));
    },
  };
}
