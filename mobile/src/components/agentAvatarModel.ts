/** Web identity rules: frontend/src/lib/assistant/nyxbot-avatar.ts. */
export const AGENT_AVATAR_TINTS = [
  {
    dark: {
      border: "rgba(166,114,251,0.30)",
      fill: "rgba(166,114,251,0.15)",
      text: "#A672FB",
    },
    light: { border: "transparent", fill: "#9555F5", text: "#FFFFFF" },
  },
  {
    dark: {
      border: "rgba(59,130,246,0.30)",
      fill: "rgba(59,130,246,0.10)",
      text: "#3B82F6",
    },
    light: { border: "transparent", fill: "#2563EB", text: "#FFFFFF" },
  },
  {
    dark: {
      border: "rgba(16,185,129,0.30)",
      fill: "rgba(16,185,129,0.10)",
      text: "#10B981",
    },
    light: { border: "transparent", fill: "#059669", text: "#FFFFFF" },
  },
  {
    dark: {
      border: "rgba(245,158,11,0.30)",
      fill: "rgba(245,158,11,0.10)",
      text: "#F59E0B",
    },
    light: { border: "transparent", fill: "#B45309", text: "#FFFFFF" },
  },
  {
    dark: {
      border: "rgba(151,97,248,0.30)",
      fill: "rgba(151,97,248,0.15)",
      text: "#B894FB",
    },
    light: { border: "transparent", fill: "#5A2AF1", text: "#FFFFFF" },
  },
  {
    dark: {
      border: "rgba(255,255,255,0.15)",
      fill: "#2C2C30",
      text: "#BABAC1",
    },
    light: { border: "#3F3F46", fill: "transparent", text: "#3F3F46" },
  },
] as const;

export const AGENT_AVATAR_SIZES = {
  xs: { circle: 16, font: 9 },
  sm: { circle: 20, font: 9 },
  md: { circle: 24, font: 9 },
  lg: { circle: 32, font: 11 },
  xl: { circle: 44, font: 13 },
} as const;

export type AgentIdentity = {
  readonly id: string;
  readonly name: string;
  readonly kind: "nyxbot" | "specialist";
  readonly display_name?: string | null;
};

export function agentTintIndex(id: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < id.length; index += 1) {
    hash ^= id.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0) % AGENT_AVATAR_TINTS.length;
}

export function agentInitials(name: string): string {
  const words = name.split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (words.length >= 2)
    return `${words[0]![0]!}${words[1]![0]!}`.toUpperCase();
  return (words[0] ?? name).slice(0, 2).toUpperCase() || "?";
}
