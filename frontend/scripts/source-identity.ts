import type { Plugin } from "vite";

/** Official exported builds supply both inputs. Ordinary dev builds stay identifiable as unverified. */
export function frontendSourceIdentity(env: Record<string, string | undefined> = process.env) {
  const explicit = env.SOURCE_COMMIT !== undefined || env.NYXID_GIT_HASH !== undefined;
  const commit = env.SOURCE_COMMIT ?? env.NYXID_GIT_HASH ?? env.GITHUB_SHA ?? null;
  for (const value of [env.SOURCE_COMMIT, env.NYXID_GIT_HASH, env.GITHUB_SHA]) {
    if (value !== undefined && (!/^[0-9a-f]{40}$/.test(value) || value !== commit)) {
      throw new Error("Conflicting or invalid full frontend source commit");
    }
  }
  const value = env.SOURCE_DATE_EPOCH;
  if ((explicit && value === undefined) || (value !== undefined &&
      (!/^[1-9][0-9]*$/.test(value) || !commit || !Number.isSafeInteger(Number(value)) || Number(value) > 253402300799))) {
    throw new Error("Frontend exported source requires a canonical commit epoch");
  }
  return {
    schema: "nyxid.frontend-build-identity.v1", commit,
    source_date_epoch: value === undefined ? null : Number(value),
    source_state: explicit ? "exported" : "unverified",
  };
}

export function sourceIdentityPlugin(): Plugin {
  const identity = frontendSourceIdentity();
  return {
    name: "nyxid-source-identity", apply: "build",
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "build-identity.json", source: JSON.stringify(identity) + "\n" });
    },
  };
}
