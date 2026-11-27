import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  createdAvatarEvent,
  claimCreatedAvatar,
  subscribeCreatedAvatars,
} from "../../../../../shared/agent-avatars/created";
import type { AvatarMood } from "@/lib/assistant/agent-avatar-model";

/** A real creation can be acknowledged by one focused avatar, once. */
export function useCreatedAvatarMood(
  agentId: string | undefined,
  mood: AvatarMood,
  eligible: boolean,
) {
  const event = useSyncExternalStore(
    subscribeCreatedAvatars,
    () => createdAvatarEvent(agentId),
    () => null,
  );
  const owner = useRef(Symbol("natural-avatar"));
  const [claimed, setClaimed] = useState<string | null>(null);
  const yielded = useRef<string | null>(null);
  useEffect(() => {
    if (!event) {
      setClaimed(null);
      return;
    }
    if (yielded.current === event.id) {
      setClaimed(null);
      return;
    }
    if (mood !== "idle") {
      if (
        (eligible || claimed === event.id) &&
        claimCreatedAvatar(event, owner.current)
      )
        yielded.current = event.id;
      setClaimed(null);
      return;
    }
    if (!eligible) return;
    setClaimed(claimCreatedAvatar(event, owner.current) ? event.id : null);
  }, [event, eligible, mood, claimed]);
  return mood === "idle" && claimed !== null && event?.id === claimed
    ? "created"
    : mood;
}
