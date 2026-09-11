import {
  configuredConnectAttemptSchema,
  type ConfiguredConnectAttempt,
} from "@/schemas/connect-links";

function storageKey(userId: string): string {
  return `nyxid:configured-connect:${userId}`;
}

function readAttempts(userId: string): ConfiguredConnectAttempt[] {
  const raw = sessionStorage.getItem(storageKey(userId));
  if (!raw) return [];
  try {
    const entries: unknown = JSON.parse(raw);
    if (!Array.isArray(entries)) return [];
    return entries.flatMap((entry) => {
      const parsed = configuredConnectAttemptSchema.safeParse(entry);
      return parsed.success ? [parsed.data] : [];
    });
  } catch {
    return [];
  }
}

export function restoreConfiguredConnectAttempt(
  userId: string,
  serviceSlug: string,
  context: string,
  returnedIds: readonly string[],
): ConfiguredConnectAttempt | null {
  const attempts = readAttempts(userId);
  const returned =
    returnedIds.length === 1
      ? attempts.find((attempt) => attempt.id === returnedIds[0])
      : undefined;
  if (returned?.service_slug === serviceSlug) return returned;
  return (
    attempts
      .reverse()
      .find(
        (attempt) =>
          attempt.service_slug === serviceSlug && attempt.context === context,
      ) ?? null
  );
}

export function saveConfiguredConnectAttempt(
  userId: string,
  attempt: ConfiguredConnectAttempt,
): void {
  const attempts = readAttempts(userId).filter(
    (saved) => saved.id !== attempt.id,
  );
  sessionStorage.setItem(
    storageKey(userId),
    JSON.stringify([...attempts, attempt]),
  );
}

export function removeConfiguredConnectAttempt(
  userId: string,
  id: string,
): void {
  const attempts = readAttempts(userId).filter((attempt) => attempt.id !== id);
  if (attempts.length === 0) sessionStorage.removeItem(storageKey(userId));
  else sessionStorage.setItem(storageKey(userId), JSON.stringify(attempts));
}
