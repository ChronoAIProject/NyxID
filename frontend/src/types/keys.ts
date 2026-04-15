import type { CredentialSource } from "@/schemas/orgs";

export interface KeyInfo {
  readonly id: string;
  readonly label: string;
  readonly slug: string;
  readonly endpoint_url: string;
  readonly endpoint_id: string;
  readonly api_key_id?: string | null;
  readonly credential_type: string;
  readonly auth_method: string;
  readonly auth_key_name: string;
  readonly status: string;
  readonly catalog_service_id: string | null;
  readonly catalog_service_name: string | null;
  readonly node_id: string | null;
  readonly node_priority: number;
  readonly is_active: boolean;
  readonly custom_user_agent?: string | null;
  readonly auto_connected: boolean;
  readonly source_app_id?: string | null;
  readonly source_app_name?: string | null;
  readonly expires_at: string | null;
  readonly last_used_at: string | null;
  readonly error_message: string | null;
  readonly created_at: string;
  readonly service_type: string;
  readonly ssh_host: string | null;
  readonly ssh_port: number | null;
  readonly ssh_ca_public_key: string | null;
  readonly ssh_allowed_principals: readonly string[] | null;
  readonly ssh_certificate_ttl_minutes: number | null;
  /**
   * Optional OpenAPI spec URL stored on the underlying user endpoint. When
   * present, AI agent surfaces (MCP, `/endpoints/{id}/openapi-endpoints`)
   * expose concrete operations parsed from the spec instead of only a
   * generic proxy tool.
   */
  readonly openapi_spec_url?: string | null;
  /**
   * Provenance of the credential backing this key. Personal items are owned
   * directly; org items are inherited from an org membership. When undefined
   * (older backends without the org model) consumers should assume
   * `{ type: "personal" }`. When the user is a viewer of the owning org,
   * `credential_source.allowed` is `false` and the UI must render the item
   * as read-only.
   */
  readonly credential_source?: CredentialSource;
}

export interface KeyListResponse {
  readonly keys: readonly KeyInfo[];
}

export interface ExternalApiKeyInfo {
  readonly id: string;
  readonly label: string;
  readonly credential_type: string;
  readonly status: string;
  readonly provider_config_id: string | null;
  readonly expires_at: string | null;
  readonly last_used_at: string | null;
  readonly error_message: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface ExternalApiKeyListResponse {
  readonly api_keys: readonly ExternalApiKeyInfo[];
}

/** Credential field declared by a `token_exchange` catalog service.
 *  Drives the dynamic multi-field credential form. */
export interface CredentialFieldSpec {
  readonly name: string;
  readonly label: string;
  readonly placeholder?: string | null;
  readonly secret: boolean;
}

export interface CatalogEntry {
  readonly slug: string;
  readonly name: string;
  readonly description: string | null;
  readonly base_url: string;
  readonly auth_method: string;
  readonly auth_key_name: string;
  readonly provider_config_id: string | null;
  readonly provider_type: string | null;
  readonly requires_gateway_url: boolean;
  readonly credential_mode: string | null;
  readonly api_key_instructions: string | null;
  readonly api_key_url: string | null;
  readonly icon_url: string | null;
  readonly documentation_url: string | null;
  readonly service_type: string;
  readonly ssh_host: string | null;
  readonly ssh_port: number | null;
  readonly ssh_ca_public_key: string | null;
  readonly ssh_allowed_principals: readonly string[] | null;
  readonly ssh_certificate_ttl_minutes: number | null;
  readonly authorization_url: string | null;
  readonly token_url: string | null;
  readonly device_code_url: string | null;
  readonly default_scopes: readonly string[] | null;
  readonly supports_pkce: boolean | null;
  /** "rfc8628" (default) or "openai". Determines whether a device-code
   *  provider accepts a `scope` parameter. OpenAI-format providers do not. */
  readonly device_code_format: string | null;
  readonly oauth_client_id: string | null;
  readonly client_id_param_name: string | null;
  readonly requires_credential: boolean;
  /** Declared credential fields for `token_exchange` services. When set,
   *  the dialog renders one input per field (text vs password depending on
   *  `secret`) and composes a JSON object from the values before submit. */
  readonly token_exchange_credential_fields?:
    | readonly CredentialFieldSpec[]
    | null;
}

export interface CatalogListResponse {
  readonly entries: readonly CatalogEntry[];
}

export interface UserEndpointInfo {
  readonly id: string;
  readonly label: string;
  readonly url: string;
  readonly catalog_service_id: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface UserServiceInfo {
  readonly id: string;
  readonly slug: string;
  readonly endpoint_id: string;
  readonly api_key_id?: string | null;
  readonly auth_method: string;
  readonly auth_key_name: string;
  readonly catalog_service_id: string | null;
  readonly node_id: string | null;
  readonly node_priority: number;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface AllowedServiceInfo {
  readonly id: string;
  readonly slug: string;
  readonly label: string;
  readonly catalog_service_name: string | null;
}

export interface AllowedNodeInfo {
  readonly id: string;
  readonly name: string;
  readonly status: string;
}

export interface NyxIdApiKeyInfo {
  readonly id: string;
  readonly name: string;
  readonly description: string | null;
  readonly key_prefix: string;
  readonly scopes: string;
  readonly last_used_at: string | null;
  readonly expires_at: string | null;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly allowed_service_ids: readonly string[];
  readonly allowed_node_ids: readonly string[];
  readonly allow_all_services: boolean;
  readonly allow_all_nodes: boolean;
  readonly allowed_services: readonly AllowedServiceInfo[];
  readonly allowed_nodes: readonly AllowedNodeInfo[];
  readonly platform: string | null;
  readonly rate_limit_per_second: number | null;
  readonly rate_limit_burst: number | null;
  readonly bindings_count: number;
  /**
   * Provenance: whether this NyxID API key is owned directly by the caller
   * (Personal) or by an org the caller belongs to (Org). Used by the
   * scope and binding pickers to filter user services to those owned by
   * the same owner -- a personal API key only binds to personal services,
   * an org API key only binds to that org's services.
   */
  readonly credential_source?: import("@/schemas/orgs").CredentialSource;
}

export interface AgentServiceBinding {
  readonly id: string;
  readonly api_key_id: string;
  readonly user_service_id: string;
  readonly user_api_key_id: string;
  readonly service_slug: string;
  readonly service_label: string;
  readonly credential_label: string;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface AgentServiceBindingListResponse {
  readonly bindings: readonly AgentServiceBinding[];
}
