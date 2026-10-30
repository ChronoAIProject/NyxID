type LinkEnvironment = {
  appScheme: string;
  apiBaseUrl: string;
  frontendUrl: string;
  universalLinkHost: string;
  allowHttp: boolean;
};

export function buildLinkingPrefixes(env: LinkEnvironment): string[] {
  const prefixes = new Set([`${env.appScheme}://`]);
  const urls = [env.apiBaseUrl, env.frontendUrl];
  if (env.universalLinkHost) {
    urls.push(`https://${env.universalLinkHost}`);
    if (env.allowHttp) urls.push(`http://${env.universalLinkHost}`);
  }
  for (const value of urls) {
    try {
      const url = new URL(value);
      if (url.protocol === "https:" || (env.allowHttp && url.protocol === "http:")) {
        // The slash prevents a configured host from matching a hostname suffix.
        prefixes.add(`${url.origin}/`);
      }
    } catch {
      // Empty or invalid optional URLs do not register a navigation prefix.
    }
  }
  return [...prefixes];
}
