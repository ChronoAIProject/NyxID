import type { NyxIdApiError, NyxIdPluginConfig, TokenProfile, ToolContext } from "./types.js";
export declare function normalizeBaseUrl(input: string): string;
export declare function normalizeConfig(input: Partial<NyxIdPluginConfig> | undefined): NyxIdPluginConfig;
export declare function requireOAuthClient(config: NyxIdPluginConfig): asserts config is NyxIdPluginConfig & {
    clientId: string;
};
export declare function createPkcePair(): {
    verifier: string;
    challenge: string;
};
export declare function createOpaqueState(): string;
export declare function buildAuthorizeUrl(config: NyxIdPluginConfig, input: {
    redirectUri: string;
    state: string;
    challenge: string;
    scope?: string;
}): string;
export declare function buildProxyUrl(baseUrl: string, service: string, apiPath: string): string;
export declare function decodeJwtPayload(token: string): Record<string, unknown> | null;
export declare function isTokenFresh(token: string | undefined, expiresAt: number | undefined): boolean;
export declare function computeExpiryTimestamp(expiresInSeconds: number): number;
export declare function nowEpochSeconds(): number;
export declare function mapNyxIdError(error: NyxIdApiError): string;
export declare function asNyxIdError(value: unknown): NyxIdApiError | null;
export declare function loadProfile(context: ToolContext, config?: NyxIdPluginConfig): Promise<TokenProfile>;
export declare function saveProfile(context: ToolContext, profile: TokenProfile): Promise<void>;
