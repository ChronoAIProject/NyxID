import { z } from "zod";
import { requestJson } from "./http";
import {
  catalogEntrySchema,
  serviceConnectionSchema,
} from "../../features/services/serviceContracts";

const keyPath = (id: string) => `/keys/${encodeURIComponent(id)}`;

export const servicesApi = {
  async pluginConnections(signal?: AbortSignal) {
    // Match Web Plugins: tool bindings belong to service management.
    const response = z
      .object({ keys: z.array(serviceConnectionSchema) })
      .parse(await requestJson("/keys", { signal }));
    return response.keys;
  },
  async connections(signal?: AbortSignal) {
    const response = z
      .object({ keys: z.array(serviceConnectionSchema) })
      .parse(await requestJson("/keys?include_tool_bindings=true", { signal }));
    return response.keys;
  },
  async catalog(signal?: AbortSignal) {
    const response = z
      .object({ entries: z.array(catalogEntrySchema) })
      .parse(await requestJson("/catalog", { signal }));
    return response.entries;
  },
  async catalogEntry(slug: string, signal?: AbortSignal) {
    const entry = catalogEntrySchema.parse(
      await requestJson(`/catalog/${encodeURIComponent(slug)}`, { signal }),
    );
    if (entry.slug !== slug)
      throw new Error(
        "The server returned a different service. Please reload.",
      );
    return entry;
  },
  async connection(id: string, signal?: AbortSignal) {
    const connection = serviceConnectionSchema.parse(
      await requestJson(keyPath(id), { signal }),
    );
    if (connection.id !== id)
      throw new Error(
        "The server returned a different connection. Please reload.",
      );
    return connection;
  },
  async create(body: {
    service_slug: string;
    label: string;
    credential?: string;
    endpoint_url?: string;
    use_platform_key?: boolean;
  }) {
    return serviceConnectionSchema.parse(
      await requestJson("/keys", { method: "POST", body }),
    );
  },
  async setActive(id: string, is_active: boolean) {
    return serviceConnectionSchema.parse(
      await requestJson(keyPath(id), { method: "PUT", body: { is_active } }),
    );
  },
  async replaceCredential(id: string, credential: string) {
    return serviceConnectionSchema.parse(
      await requestJson(keyPath(id), { method: "PUT", body: { credential } }),
    );
  },
  async remove(id: string, tokenOnly = false) {
    return requestJson<{
      deleted: boolean;
      message: string;
      upstream_revocation_scheduled: boolean;
    }>(`${keyPath(id)}${tokenOnly ? "?grant_scope=token" : ""}`, {
      method: "DELETE",
    });
  },
  async initiateOAuth(
    providerId: string,
    connectionId: string,
    orgId?: string,
    scopeOverride?: readonly string[],
  ) {
    const query = new URLSearchParams({ key_id: connectionId });
    if (orgId) query.set("target_org_id", orgId);
    // Like Web, defined (including empty) replaces the provider's defaults.
    if (scopeOverride !== undefined)
      query.set("scope_override", scopeOverride.join(","));
    return z
      .object({ authorization_url: z.string() })
      .parse(
        await requestJson(
          `/providers/${encodeURIComponent(providerId)}/connect/oauth?${query.toString()}`,
        ),
      );
  },
};
