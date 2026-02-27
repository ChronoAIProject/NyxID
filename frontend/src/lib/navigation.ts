/**
 * Open a URL — used for OAuth flows and external links.
 */
export function openExternal(url: string): void {
  window.location.assign(url);
}

/**
 * Hard-redirect to a URL — for OAuth authorization_url and similar.
 */
export function hardRedirect(url: string): void {
  window.location.href = url;
}
