export interface OidcConfiguration {
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly userinfo_endpoint: string;
  readonly jwks_uri: string;
  readonly scopes_supported?: readonly string[];
  readonly response_types_supported?: readonly string[];
  readonly code_challenge_methods_supported?: readonly string[];
}

export interface EndpointParameter {
  readonly name: string;
  readonly in: 'path' | 'query' | 'header';
  readonly required: boolean;
  readonly description?: string;
  readonly schema: ParameterSchema;
}

export interface ParameterSchema {
  readonly type: string;
  readonly format?: string;
  readonly enum?: readonly string[];
  readonly default?: unknown;
  readonly description?: string;
}

export interface McpEndpoint {
  readonly id: string;
  readonly name: string;
  readonly description: string | null;
  readonly method: string;
  readonly path: string;
  readonly parameters: readonly EndpointParameter[] | null;
  readonly request_body_schema: Record<string, unknown> | null;
}

export interface McpService {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly endpoints: readonly McpEndpoint[];
}

export interface McpConfig {
  readonly services: readonly McpService[];
}

export interface UserInfo {
  readonly sub: string;
  readonly email?: string;
  readonly name?: string;
}

export interface ProtectedResourceMetadata {
  readonly resource: string;
  readonly authorization_servers: readonly string[];
  readonly scopes_supported: readonly string[];
  readonly bearer_methods_supported: readonly string[];
}
