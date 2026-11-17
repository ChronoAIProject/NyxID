import { QueryClient } from "@tanstack/react-query";
import type { KeyInfo } from "../../src/types/keys";

// Synthetic account and keys; browser routes supply every API response.
export const client = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});
export const queryKey = ["keys", "list", "fixture"];
const base = {
  is_active: true,
  status: "active",
  credential_type: "api_key",
  api_key_id: "fixture-credential",
  credential_source: { type: "personal" },
  catalog_service_id: "documents",
  catalog_service_name: "Documents",
} as KeyInfo;
export const keys: KeyInfo[] = [
  {
    ...base,
    id: "unavailable",
    slug: "unavailable",
    label: "Missing credential",
    credential_missing: true,
  },
  { ...base, id: "work", slug: "work", label: "Work documents" },
  { ...base, id: "personal", slug: "personal", label: "Personal documents" },
  {
    ...base,
    id: "unverified",
    slug: "unverified",
    label: "Unverified account",
    api_key_id: null,
    credential_source: undefined,
  },
  {
    ...base,
    id: "oauth",
    slug: "oauth",
    label: "Refreshable OAuth",
    credential_type: "oauth2",
    expires_at: "2020-01-01",
  },
  {
    ...base,
    id: "fallback",
    slug: "fallback",
    label: "Node with fallback",
    node_id: "node",
    node_status: "offline",
  },
  {
    ...base,
    id: "offline",
    slug: "offline",
    label: "Node without fallback",
    node_id: "node",
    node_status: "offline",
    execution_unavailable_reason: "node home offline",
  },
  {
    ...base,
    id: "platform",
    slug: "platform",
    label: "Unavailable platform",
    credential_binding: "platform",
    platform_key_available: false,
  },
];
export let serverConnections: KeyInfo[];

export function restoreConnections() {
  serverConnections = structuredClone(keys);
  client.setQueryData(queryKey, serverConnections);
}
restoreConnections();

export function updateConnection(id: string, patch: Partial<KeyInfo>) {
  serverConnections = serverConnections.map((key) =>
    key.id === id ? { ...key, ...patch } : key,
  );
  client.setQueryData<readonly KeyInfo[]>(queryKey, (current) =>
    current?.map((key) => (key.id === id ? { ...key, ...patch } : key)),
  );
}
