import type { KeyInfo } from "@/types/keys";

export function connectionOwnerLabel(connection: KeyInfo): string {
  return connection.credential_source?.type === "org"
    ? connection.credential_source.org_name
    : "Personal";
}

export function connectionAvailable(connection: KeyInfo): boolean {
  return (
    connection.is_active &&
    !(
      connection.credential_source?.type === "org" &&
      !connection.credential_source.allowed
    )
  );
}
