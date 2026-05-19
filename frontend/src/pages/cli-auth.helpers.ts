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
