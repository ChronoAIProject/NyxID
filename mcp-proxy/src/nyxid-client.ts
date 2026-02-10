import type { Config } from './config.js';
import type { McpConfig, OidcConfiguration, UserInfo } from './types.js';

export class NyxIdClient {
  private readonly baseUrl: string;
  private oidcConfig: OidcConfiguration | null = null;

  constructor(private readonly config: Config) {
    this.baseUrl = config.nyxidUrl.replace(/\/$/, '');
  }

  async discoverOidc(): Promise<OidcConfiguration> {
    if (this.oidcConfig) {
      return this.oidcConfig;
    }

    const response = await fetch(
      `${this.baseUrl}/.well-known/openid-configuration`,
    );

    if (!response.ok) {
      throw new Error(
        `OIDC discovery failed: ${response.status} ${response.statusText}`,
      );
    }

    const config = (await response.json()) as OidcConfiguration;
    this.oidcConfig = config;
    return config;
  }

  async getUserInfo(accessToken: string): Promise<UserInfo> {
    const oidc = await this.discoverOidc();

    const response = await fetch(oidc.userinfo_endpoint, {
      headers: { Authorization: `Bearer ${accessToken}` },
    });

    if (!response.ok) {
      throw new Error(`UserInfo request failed: ${response.status}`);
    }

    return (await response.json()) as UserInfo;
  }

  async getMcpConfig(accessToken: string): Promise<McpConfig> {
    const response = await fetch(`${this.baseUrl}/api/v1/mcp/config`, {
      headers: { Authorization: `Bearer ${accessToken}` },
    });

    if (!response.ok) {
      throw new Error(`MCP config request failed: ${response.status}`);
    }

    return (await response.json()) as McpConfig;
  }

  async proxyRequest(
    accessToken: string,
    serviceId: string,
    method: string,
    path: string,
    query?: Record<string, string>,
    body?: unknown,
  ): Promise<{
    readonly status: number;
    readonly headers: Record<string, string>;
    readonly body: unknown;
  }> {
    const url = new URL(
      `${this.baseUrl}/api/v1/proxy/${serviceId}/${path}`,
    );

    if (query) {
      for (const [key, value] of Object.entries(query)) {
        url.searchParams.set(key, value);
      }
    }

    const headers: Record<string, string> = {
      Authorization: `Bearer ${accessToken}`,
    };

    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
    }

    const response = await fetch(url.toString(), {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });

    const responseHeaders: Record<string, string> = {};
    response.headers.forEach((value, key) => {
      responseHeaders[key] = value;
    });

    const contentType = response.headers.get('content-type') ?? '';
    const responseBody = contentType.includes('application/json')
      ? await response.json()
      : await response.text();

    return {
      status: response.status,
      headers: responseHeaders,
      body: responseBody,
    };
  }
}
