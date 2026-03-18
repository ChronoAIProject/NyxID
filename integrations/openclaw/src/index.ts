import {
  ensureBaseToken,
  ensureDelegatedToken,
  exchangeAuthorizationCode,
  getProxyCredentialProfile,
  listServices,
  proxyRequest,
} from "./client.js";
import {
  buildAuthorizeUrl,
  createOpaqueState,
  createPkcePair,
  loadProfile,
  normalizeConfig,
  requireOAuthClient,
  saveProfile,
} from "./helpers.js";
import type { OpenClawPluginApi, TokenProfile, ToolContext } from "./types.js";

declare global {
  var OPENCLAW_NYXID_CONFIG: Record<string, unknown> | undefined;
}

function getFetch(context?: ToolContext): typeof fetch {
  return context?.fetch ?? fetch;
}

function mergeProfile(current: TokenProfile, updates: TokenProfile): TokenProfile {
  return { ...current, ...updates };
}

export default function register(api: OpenClawPluginApi): void {
  const registerProvider = api.registerProvider ?? api.registerAuthProvider;
  if (registerProvider) {
    registerProvider({
      id: "nyxid",
      name: "NyxID",
      type: "oauth2",
      authorize: async ({ redirectUri, state, scope }) => {
        const config = normalizeConfig(globalThis.OPENCLAW_NYXID_CONFIG as Record<string, unknown>);
        requireOAuthClient(config);

        const pkce = createPkcePair();
        const finalState = state || createOpaqueState();
        return {
          authorizationUrl: buildAuthorizeUrl(config, {
            redirectUri,
            state: finalState,
            challenge: pkce.challenge,
            scope,
          }),
          verifier: pkce.verifier,
          state: finalState,
        };
      },
      exchangeCode: async ({ code, redirectUri, codeVerifier }) => {
        const config = normalizeConfig(globalThis.OPENCLAW_NYXID_CONFIG as Record<string, unknown>);
        return exchangeAuthorizationCode(getFetch(), config, {
          code,
          redirectUri,
          codeVerifier,
        });
      },
      refresh: async (token) => {
        const config = normalizeConfig(globalThis.OPENCLAW_NYXID_CONFIG as Record<string, unknown>);
        const refreshed = await ensureBaseToken(getFetch(), config, token);
        if (!refreshed.accessToken) {
          throw new Error("NyxID refresh requires an OAuth token profile.");
        }

        return {
          access_token: refreshed.accessToken,
          token_type: refreshed.tokenType || "Bearer",
          expires_in: Math.max((refreshed.accessTokenExpiresAt || 0) - Math.floor(Date.now() / 1000), 0),
          refresh_token: refreshed.refreshToken,
          scope: refreshed.scope,
        };
      },
      tokenExchange: async (token) => {
        const config = normalizeConfig(globalThis.OPENCLAW_NYXID_CONFIG as Record<string, unknown>);
        const delegated = await ensureDelegatedToken(getFetch(), config, token);
        const proxyProfile = getProxyCredentialProfile(delegated);
        if (!proxyProfile.accessToken) {
          throw new Error("NyxID delegated proxy access requires an OAuth token profile.");
        }

        return {
          access_token: proxyProfile.accessToken,
          token_type: delegated.tokenType || "Bearer",
          expires_in: Math.max((delegated.delegatedAccessTokenExpiresAt || delegated.accessTokenExpiresAt || 0) - Math.floor(Date.now() / 1000), 0),
          scope: config.delegationScopes,
        };
      },
    });
  }

  api.registerTool?.({
    name: "nyxid_list_services",
    description: "List services available through the current user's NyxID account.",
    execute: async (_params, context) => {
      const config = normalizeConfig(context.config);
      const profile = await loadProfile(context, config);
      const updatedProfile = await ensureBaseToken(getFetch(context), config, profile);
      await saveProfile(context, updatedProfile);
      const response = await listServices(getFetch(context), config, updatedProfile);
      return {
        services: response.services.map((service) => ({
          id: service.id,
          slug: service.slug,
          name: service.name,
          connected: service.connected,
          requires_connection: service.requires_connection,
          proxy_url_slug: service.proxy_url_slug,
        })),
        total: response.total,
        page: response.page,
        per_page: response.per_page,
      };
    },
  });

  api.registerTool?.({
    name: "nyxid_proxy",
    description: "Call a user-connected external service through the NyxID proxy.",
    parameters: {
      type: "object",
      properties: {
        service: { type: "string", description: "NyxID service slug such as twitter or github" },
        method: {
          type: "string",
          enum: ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"],
          description: "HTTP method to send to the downstream service",
        },
        path: { type: "string", description: "Downstream API path such as /2/tweets" },
        body: { type: "object", description: "Optional JSON request body" },
      },
      required: ["service", "method", "path"],
    },
    execute: async (params, context) => {
      const config = normalizeConfig(context.config);
      const profile = await loadProfile(context, config);
      const proxyReadyProfile = await ensureDelegatedToken(getFetch(context), config, profile);
      await saveProfile(context, mergeProfile(profile, proxyReadyProfile));
      return proxyRequest(getFetch(context), config, {
        profile: getProxyCredentialProfile(proxyReadyProfile),
        service: String(params.service),
        method: String(params.method || "GET"),
        path: String(params.path),
        body: params.body,
      });
    },
  });
}

export * from "./client.js";
export * from "./helpers.js";
export * from "./types.js";
