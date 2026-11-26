const { paths: claimedPaths } = require("../../../shared/native-app-links.json");

// The selected host list is the only rollout input. The shared path manifest
// and native route code stay identical for API-only and API-plus-frontend sets.
function buildNativeLinks({ associatedUrls, universalLinkHost, universalLinkPathPrefix, scheme, androidHttpsAppLinks = false }) {
  const associatedDomains = [];
  for (const associatedUrl of associatedUrls) {
    try {
      const domain = `applinks:${new URL(associatedUrl).host}`;
      if (!associatedDomains.includes(domain)) associatedDomains.push(domain);
    } catch {
      // Invalid optional URLs do not register a native association.
    }
  }
  if (universalLinkHost) associatedDomains.push(`applinks:${universalLinkHost}`);

  const androidIntentFilters = [{
    action: "VIEW",
    data: [{ scheme }],
    category: ["BROWSABLE", "DEFAULT"],
  }];
  if (androidHttpsAppLinks) {
    const hosts = new Map();
    for (const value of associatedUrls) {
      try {
        const url = new URL(value);
        if (url.protocol === "https:") hosts.set(url.host, {
          host: url.hostname,
          ...(url.port ? { port: url.port } : {}),
        });
      } catch {
        // Invalid optional URLs do not register a native association.
      }
    }
    for (const authority of hosts.values()) {
      androidIntentFilters.push({
        action: "VIEW",
        autoVerify: true,
        data: claimedPaths.map((path) => ({
          scheme: "https",
          ...authority,
          ...(path.includes("*")
            ? { pathPrefix: path.slice(0, -1) }
            : { path }),
        })),
        category: ["BROWSABLE", "DEFAULT"],
      });
    }
  }
  if (universalLinkHost) {
    const data = { scheme: "https", host: universalLinkHost };
    if (universalLinkPathPrefix) data.pathPrefix = universalLinkPathPrefix;
    androidIntentFilters.unshift({
      action: "VIEW",
      autoVerify: true,
      data: [data],
      category: ["BROWSABLE", "DEFAULT"],
    });
  }
  return { associatedDomains, androidIntentFilters };
}

module.exports = { buildNativeLinks };
