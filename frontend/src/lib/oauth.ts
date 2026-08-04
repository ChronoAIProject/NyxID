export interface ParseRedirectUrisResult {
  readonly uris: string[];
  readonly error?: string;
}

export function parseRedirectUris(input: string): ParseRedirectUrisResult {
  const candidates = input
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);

  if (candidates.length === 0) {
    return { uris: [], error: "At least one redirect URI is required" };
  }

  const uniqueUris = new Set<string>();
  const uris: string[] = [];

  for (const uri of candidates) {
    let parsed: URL;
    try {
      parsed = new URL(uri);
    } catch {
      return { uris: [], error: `Invalid redirect URI format: ${uri}` };
    }

    if (
      parsed.protocol === "javascript:" ||
      parsed.protocol === "data:" ||
      parsed.protocol === "file:"
    ) {
      return { uris: [], error: `Unsupported redirect URI scheme: ${uri}` };
    }

    if (!uniqueUris.has(uri)) {
      uniqueUris.add(uri);
      uris.push(uri);
    }
  }

  return { uris };
}
