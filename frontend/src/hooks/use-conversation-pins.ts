import { useMemo } from "react";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantPinsStore } from "@/stores/assistant-pins-store";

const NO_PINS: readonly string[] = [];

export function usePinnedConversationIds(): ReadonlySet<string> {
  const userId = useAuthStore((state) => state.user?.id);
  const ids = useAssistantPinsStore((state) =>
    userId && state.ownerUserId === userId ? state.ids : NO_PINS,
  );
  return useMemo(() => new Set(ids), [ids]);
}
