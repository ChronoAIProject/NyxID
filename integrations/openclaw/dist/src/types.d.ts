export interface NyxIdPluginConfig {
    baseUrl: string;
    clientId?: string;
    clientSecret?: string;
    defaultScopes?: string;
    delegationScopes?: string;
    apiKey?: string;
}
export interface OAuthTokenSet {
    access_token: string;
    token_type: string;
    expires_in: number;
    refresh_token?: string;
    id_token?: string;
    scope?: string;
}
export interface DelegatedTokenSet {
    access_token: string;
    token_type: string;
    expires_in: number;
    scope?: string;
}
export interface NyxIdService {
    id: string;
    name: string;
    slug: string;
    description: string | null;
    service_category: string;
    connected: boolean;
    requires_connection: boolean;
    proxy_url: string;
    proxy_url_slug: string;
}
export interface ServiceListResponse {
    services: NyxIdService[];
    total: number;
    page: number;
    per_page: number;
}
export interface NyxIdApiError {
    error: string;
    error_code: number;
    message: string;
    consent_url?: string;
}
export interface TokenProfile {
    accessToken?: string;
    refreshToken?: string;
    delegatedAccessToken?: string;
    accessTokenExpiresAt?: number;
    delegatedAccessTokenExpiresAt?: number;
    tokenType?: string;
    scope?: string;
    apiKey?: string;
}
export interface OpenClawProviderRegistration {
    id: string;
    name: string;
    type: string;
    authorize: (input: {
        redirectUri: string;
        state?: string;
        scope?: string;
    }) => Promise<{
        authorizationUrl: string;
        verifier: string;
        state: string;
    }>;
    exchangeCode: (input: {
        code: string;
        redirectUri: string;
        codeVerifier: string;
    }) => Promise<OAuthTokenSet>;
    refresh: (token: TokenProfile) => Promise<OAuthTokenSet>;
    tokenExchange: (token: TokenProfile) => Promise<DelegatedTokenSet>;
}
export interface OpenClawToolRegistration {
    name: string;
    description: string;
    parameters?: Record<string, unknown>;
    execute: (params: Record<string, unknown>, context: ToolContext) => Promise<unknown>;
}
export interface OpenClawPluginApi {
    registerProvider?: (provider: OpenClawProviderRegistration) => void;
    registerAuthProvider?: (provider: OpenClawProviderRegistration) => void;
    registerTool?: (tool: OpenClawToolRegistration) => void;
}
export interface ToolContext {
    config?: Partial<NyxIdPluginConfig>;
    env?: Record<string, string | undefined>;
    auth?: {
        profile?: TokenProfile;
        saveProfile?: (profile: TokenProfile) => Promise<void> | void;
        getProviderProfile?: (providerId: string) => Promise<TokenProfile | null> | TokenProfile | null;
    };
    getProviderProfile?: (providerId: string) => Promise<TokenProfile | null> | TokenProfile | null;
    saveProviderProfile?: (providerId: string, profile: TokenProfile) => Promise<void> | void;
    fetch?: typeof fetch;
}
