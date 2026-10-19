export interface BuildVersion {
  readonly buildId: string;
  readonly commit: string | null;
  readonly assets: readonly string[];
  readonly assistant?: string;
}

/** Shared by the producer and browser so unsupported metadata cannot ship. */
export function parseBuildVersion(value: unknown): BuildVersion | null {
  if (!value || typeof value !== "object") return null;
  const version = value as Partial<BuildVersion>;
  if (
    typeof version.buildId !== "string" || !version.buildId ||
    version.buildId.length > 128 ||
    !Array.isArray(version.assets) || version.assets.length === 0 ||
    version.assets.length > 256 ||
    !version.assets.every((asset) => typeof asset === "string" &&
      /^assets\/[a-zA-Z0-9_.-]+\.(js|css)$/.test(asset))
  ) return null;
  return {
    buildId: version.buildId,
    commit: typeof version.commit === "string" ? version.commit : null,
    assets: version.assets,
    ...(typeof version.assistant === "string" && /^[a-f0-9]{64}$/.test(version.assistant)
      ? { assistant: version.assistant } : {}),
  };
}
