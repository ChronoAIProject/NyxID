export function createIdempotencyKey(scope: string, entityId: string): string {
  const randomPart = Math.random().toString(36).slice(2, 10);
  return `${scope}:${entityId}:${Date.now()}:${randomPart}`;
}
