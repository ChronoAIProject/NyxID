export interface CliAuthSearch {
  readonly port?: string;
  readonly state?: string;
  readonly client_ua?: string;
}

export function buildCliAuthReturnPath(search: CliAuthSearch): string {
  const params = new URLSearchParams();

  if (search.port) {
    params.set("port", search.port);
  }
  if (search.state) {
    params.set("state", search.state);
  }
  if (search.client_ua) {
    params.set("client_ua", search.client_ua);
  }

  const query = params.toString();
  return query ? `/cli-auth?${query}` : "/cli-auth";
}

export function cliCallbackUrl(port: unknown): URL | undefined {
  if (typeof port !== "string" && typeof port !== "number") return;
  const value = String(port);
  if (!value || /[^0-9]/.test(value)) return;
  const number = Number(value);
  if (number < 1 || number > 65535) return;
  const url = new URL("http://127.0.0.1/callback");
  url.port = String(number);
  return url;
}

// Validate the wire spelling before the router coerces search values to JSON.
export function validateCliAuthUrl(url: URL): URL {
  const ports = url.searchParams.getAll("port");
  if (ports.length !== 1 || !cliCallbackUrl(ports[0])) {
    url.searchParams.delete("port");
  }
  return url;
}
