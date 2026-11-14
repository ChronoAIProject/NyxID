const { paths: claimedPaths } = require("../../../shared/native-app-links.json");

// Every embedded API host is associated, since the admin flag can select either at launch.
function buildNativeLinks({ apiBaseUrls, universalLinkHost, universalLinkPathPrefix, scheme, firstPartyLinkUrls = apiBaseUrls, androidApiAppLinks = false }) {
  const associatedDomains = [];
  for (const apiBaseUrl of firstPartyLinkUrls) {
    try {
      const domain = `applinks:${new URL(apiBaseUrl).host}`;
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
  if (androidApiAppLinks) {
    const hosts = new Set();
    for (const value of firstPartyLinkUrls) {
      try { const url = new URL(value); if (url.protocol === "https:") hosts.add(url.host); } catch {}
    }
    for (const host of hosts) androidIntentFilters.push({
      action: "VIEW", autoVerify: true,
      data: claimedPaths.map((path) => ({ scheme: "https", host, ...(path.includes("*") ? { pathPrefix: path.slice(0, -1) } : { path }) })),
      category: ["BROWSABLE", "DEFAULT"],
    });
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
