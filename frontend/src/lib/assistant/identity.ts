type AssistantIdentityListener = (
  userId: string | null,
  previousUserId: string | null,
) => void;

let currentUserId: string | null = null;
const listeners = new Set<AssistantIdentityListener>();

export function getAssistantIdentityUserId(): string | null {
  return currentUserId;
}

export function transitionAssistantIdentity(userId: string | null): void {
  if (userId === currentUserId) return;
  const previousUserId = currentUserId;
  currentUserId = userId;
  for (const listener of listeners) listener(userId, previousUserId);
}

export function subscribeAssistantIdentity(
  listener: AssistantIdentityListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
