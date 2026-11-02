function buildNativeLinks({ apiBaseUrl, universalLinkHost, universalLinkPathPrefix, scheme }) {
  const associatedDomains = [];
  try {
    associatedDomains.push(`applinks:${new URL(apiBaseUrl).host}`);
  } catch {
    // Invalid optional URLs do not register a native association.
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
