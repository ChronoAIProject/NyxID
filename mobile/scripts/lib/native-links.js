// Every embedded API host is associated, since the admin flag can select either at launch.
function buildNativeLinks({ apiBaseUrls, universalLinkHost, universalLinkPathPrefix, scheme }) {
  const associatedDomains = [];
  for (const apiBaseUrl of apiBaseUrls) {
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
