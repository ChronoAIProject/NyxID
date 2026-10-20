/** What an agent app may do in one service. */
export type AccessLevel = "off" | "allow";

export interface AccessServiceRow {
  readonly id: string;
  readonly name: string;
  readonly secondary: string;
  readonly orgName: string | null;
  /** The app asked for this service, so it cannot be turned off. */
  readonly requiredByApp: boolean;
}

export interface AgentGrant {
  /** Access to every service, including ones connected later. */
  readonly allowAll: boolean;
  readonly serviceIds: readonly string[];
}

/** A row's level: its override, else the default; app-requested rows stay allowed. */
export function rowLevel(
  row: AccessServiceRow,
  defaultLevel: AccessLevel,
  overrides: Readonly<Record<string, AccessLevel>>,
): AccessLevel {
  const level = overrides[row.id] ?? defaultLevel;
  return row.requiredByApp && level === "off" ? "allow" : level;
}

/**
 * Turns the table into the grant the decision endpoint stores. "All" is
 * only kept while no row is below the default, because a list cannot say
 * "everything except X" for services connected later.
 */
export function agentGrant(
  rows: readonly AccessServiceRow[],
  defaultLevel: AccessLevel,
  overrides: Readonly<Record<string, AccessLevel>>,
): AgentGrant {
  const levels = rows.map((row) => ({
    id: row.id,
    level: rowLevel(row, defaultLevel, overrides),
  }));
  const serviceIds = levels
    .filter((item) => item.level === "allow")
    .map((item) => item.id);
  return {
    allowAll:
      defaultLevel === "allow" &&
      levels.every((item) => item.level === "allow"),
    serviceIds,
  };
}
