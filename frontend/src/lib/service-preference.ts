import type { KeyInfo } from "@/types/keys";

export function preferredConnection(connections: readonly KeyInfo[]) {
  return connections.reduce<KeyInfo | undefined>((best, connection) =>
    connection.preference_rank != null &&
    (best?.preference_rank == null || connection.preference_rank < best.preference_rank)
      ? connection : best, undefined);
}
