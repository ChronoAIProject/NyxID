type AssistantIdentityListener = (
  userId: string | null,
  previousUserId: string | null,
) => void;

let currentUserId: string | null = null;
let identityRevision = 0;
const listeners = new Set<AssistantIdentityListener>();

export function getAssistantIdentityUserId(): string | null {
  return currentUserId;
}

/** Changes even when an account switches away and back during one request. */
export function getAssistantIdentityRevision(): number {
  return identityRevision;
}

export function transitionAssistantIdentity(userId: string | null): void {
  if (userId === currentUserId) return;
  const previousUserId = currentUserId;
  currentUserId = userId;
  identityRevision += 1;
  for (const listener of listeners) listener(userId, previousUserId);
}

export function subscribeAssistantIdentity(
  listener: AssistantIdentityListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
