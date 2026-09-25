export function isHostedConnectLink(href: string, origin: string): boolean {
  try {
    const url = new URL(href, origin);
    return url.origin === origin &&
      !url.username &&
      !url.password &&
      /^\/connect\/nyx_clk_[A-Za-z0-9_-]+$/.test(url.pathname) &&
      !url.search &&
      !url.hash;
  } catch {
    return false;
  }
}
