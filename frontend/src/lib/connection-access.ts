import type { KeyInfo } from "@/types/keys";

export function canEditConnection(key: KeyInfo): boolean {
  if (key.auto_connected || key.can_edit_configuration === false) return false;
  // Older servers omit the explicit permission; require known ownership.
  const source = key.credential_source;
  return (
    source?.type === "personal" ||
    (source?.type === "org" && source.role === "admin" && source.allowed)
  );
}
