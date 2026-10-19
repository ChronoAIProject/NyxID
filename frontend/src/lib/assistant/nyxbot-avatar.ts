/**
 * Identity tints for specialists, drawn from DESIGN.md tokens only. Each uses
 * the badge convention: a tuned tint on dark, a solid fill on light. The
 * destructive red is deliberately left out so an avatar never reads as an
 * error.
 */
export const AGENT_AVATAR_TINTS = [
  "border-nyx-secondary-400/30 bg-nyx-secondary-400/15 text-nyx-secondary-400 light:border-transparent light:bg-nyx-secondary-500 light:text-white",
  "border-info/30 bg-info/10 text-info light:border-transparent light:bg-info light:text-white",
  "border-success/30 bg-success/10 text-success light:border-transparent light:bg-success light:text-white",
  "border-warning/30 bg-warning/10 text-warning light:border-transparent light:bg-warning light:text-white",
  "border-nyx-300/30 bg-nyx-300/15 text-nyx-200 light:border-transparent light:bg-nyx-500 light:text-white",
  "border-hairline-strong bg-muted text-muted-foreground light:border-muted-foreground light:bg-transparent",
] as const;

/** A stable tint index for an agent id (FNV-1a over its characters). */
export function agentTintIndex(id: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < id.length; index += 1) {
    hash ^= id.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0) % AGENT_AVATAR_TINTS.length;
}

/** One or two letters from an agent name: "release-notes" -> "RN". */
export function agentInitials(name: string): string {
  const words = name.split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (words.length >= 2) return `${words[0]![0]!}${words[1]![0]!}`.toUpperCase();
  return (words[0] ?? name).slice(0, 2).toUpperCase() || "?";
}
