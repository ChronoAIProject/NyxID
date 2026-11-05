// CMA-supplied browser destinations only; preserve the supplied URL verbatim.
export function safeCmaUrl(
  value: string | null | undefined,
): string | undefined {
  if (
    !value ||
    new TextEncoder().encode(value).length > 2048 ||
    value.includes("?") ||
    value.includes("#") ||
    !/^https?:\/\//i.test(value) ||
    value.split("://")[1]?.split("/")[0]?.includes("@") ||
    /[\s\\]/u.test(value) ||
    [...value].some(
      (char) =>
        char.charCodeAt(0) < 32 ||
        (char.charCodeAt(0) >= 127 && char.charCodeAt(0) <= 159),
    )
  )
    return undefined;
  try {
    const url = new URL(value);
    const host = url.hostname
      .replace(/^\[|\]$/g, "")
      .replace(/\.$/, "")
      .toLowerCase();
    return ["https:", "http:"].includes(url.protocol) &&
      Boolean(host) &&
      !url.username &&
      !url.password &&
      ![
        "metadata.google.internal",
        "169.254.169.254",
        "fd00:ec2::254",
        "100.100.100.200",
        "168.63.129.16",
      ].includes(host)
      ? value
      : undefined;
  } catch {
    return undefined;
  }
}
