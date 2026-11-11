/** Runtime URLs are data. Deployment CSP admits separately configured origins. */
export function isHttpUrl(value: string, originOnly = false): boolean {
  try {
    const url = new URL(value);
    return /^https?:\/\/[^/?#]/i.test(value) && /^https?:$/.test(url.protocol)
      && !url.username && !url.password && !value.includes("#") && !/[\s\\]/.test(value)
      && (!originOnly || !value.includes("?"))
      && (!originOnly || ((url.pathname === "/" || url.pathname === "") && !url.search));
  } catch {
    return false;
  }
}
