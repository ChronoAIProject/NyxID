export function buildOrgInviteJoinUrl(
  nonce: string,
  currentOrigin?: string | null,
): string {
  const origin =
    arguments.length > 1
      ? currentOrigin
      : typeof window === "undefined"
        ? undefined
        : window.location.origin;
  if (!origin) return `/orgs/join/${nonce}`;
  return `${origin}/orgs/join/${nonce}`;
}
