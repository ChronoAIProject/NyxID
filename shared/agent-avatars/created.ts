/** Ephemeral evidence of a successful create in this client view. Never
 * populated from roster reads, persisted, or replayed after a reload. */
export interface CreatedAvatarEvent {
  readonly id: string;
  readonly agentId: string;
  readonly expiresAt: number;
  owner?: symbol;
}
const events = new Map<string, CreatedAvatarEvent>();
const listeners = new Set<() => void>();
let sequence = 0,
  timer: ReturnType<typeof setTimeout> | undefined;
function publish() {
  for (const listener of listeners) listener();
}
function expire() {
  timer = undefined;
  const now = Date.now();
  for (const [id, event] of events)
    if (event.expiresAt <= now) events.delete(id);
  publish();
  schedule();
}
function schedule() {
  if (timer) clearTimeout(timer);
  timer = undefined;
  if (!events.size) return;
  const next = Math.min(...[...events.values()].map((e) => e.expiresAt));
  timer = setTimeout(expire, Math.max(1, next - Date.now()));
}
export function recordCreatedAvatar(agentId: string): void {
  if (events.size >= 8) events.delete(events.keys().next().value!);
  events.set(agentId, {
    id: `${agentId}:${++sequence}`,
    agentId,
    expiresAt: Date.now() + 3000,
  });
  publish();
  schedule();
}
export function createdAvatarEvent(
  agentId: string | undefined,
): CreatedAvatarEvent | null {
  const event = agentId ? events.get(agentId) : undefined;
  return event && event.expiresAt > Date.now() ? event : null;
}
export function claimCreatedAvatar(
  event: CreatedAvatarEvent,
  owner: symbol,
): boolean {
  if (event.expiresAt <= Date.now() || (event.owner && event.owner !== owner))
    return false;
  event.owner = owner;
  return true;
}
export function subscribeCreatedAvatars(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
export function clearCreatedAvatars() {
  if (timer) clearTimeout(timer);
  timer = undefined;
  events.clear();
  publish();
}
