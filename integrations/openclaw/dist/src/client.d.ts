import type { DelegatedTokenSet, NyxIdApiError, NyxIdPluginConfig, OAuthTokenSet, ServiceListResponse, TokenProfile } from "./types.js";
type FetchImpl = typeof fetch;
export declare function buildAuthHeaders(profile: TokenProfile): Record<string, string>;
export declare function exchangeAuthorizationCode(fetchImpl: FetchImpl, config: NyxIdPluginConfig, input: {
    code: string;
    redirectUri: string;
    codeVerifier: string;
}): Promise<OAuthTokenSet>;
export declare function refreshAccessToken(fetchImpl: FetchImpl, config: NyxIdPluginConfig, refreshToken: string): Promise<OAuthTokenSet>;
export declare function exchangeDelegatedToken(fetchImpl: FetchImpl, config: NyxIdPluginConfig, accessToken: string): Promise<DelegatedTokenSet>;
export declare function refreshDelegatedToken(fetchImpl: FetchImpl, config: NyxIdPluginConfig, delegatedToken: string): Promise<DelegatedTokenSet>;
export declare function listServices(fetchImpl: FetchImpl, config: NyxIdPluginConfig, profile: TokenProfile): Promise<ServiceListResponse>;
export declare function proxyRequest(fetchImpl: FetchImpl, config: NyxIdPluginConfig, input: {
    profile: TokenProfile;
    service: string;
    method: string;
    path: string;
    body?: unknown;
}): Promise<unknown>;
export declare function ensureBaseToken(fetchImpl: FetchImpl, config: NyxIdPluginConfig, profile: TokenProfile): Promise<TokenProfile>;
export declare function ensureDelegatedToken(fetchImpl: FetchImpl, config: NyxIdPluginConfig, profile: TokenProfile): Promise<TokenProfile>;
export declare function getProxyCredentialProfile(profile: TokenProfile): TokenProfile;
export declare function describeServiceConnection(service: ServiceListResponse["services"][number]): string;
export declare function isNyxIdApiError(error: unknown): error is NyxIdApiError;
export {};
