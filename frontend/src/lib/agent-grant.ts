/** What an agent app may do in one service. */
export type AccessLevel = "off" | "read" | "write";

/** How the write half of a grant is submitted to the decision endpoint. */
export type WriteAccessMode = "none" | "all" | "selected";

export interface AccessServiceRow {
  readonly id: string;
  readonly name: string;
  readonly secondary: string;
  readonly orgName: string | null;
  /** The app asked for this service, so it cannot be turned off. */
  readonly requiredByApp: boolean;
}

export interface AgentGrant {
  /** Read access to every service, including ones connected later. */
  readonly allowAll: boolean;
  readonly readIds: readonly string[];
  readonly writeMode: WriteAccessMode;
  readonly writeIds: readonly string[];
}

const RANK: Readonly<Record<AccessLevel, number>> = {
  off: 0,
  read: 1,
  write: 2,
};

/** A row's level: its override, else the default; app-requested rows stay readable. */
export function rowLevel(
  row: AccessServiceRow,
  defaultLevel: AccessLevel,
  overrides: Readonly<Record<string, AccessLevel>>,
): AccessLevel {
  const level = overrides[row.id] ?? defaultLevel;
  return row.requiredByApp && level === "off" ? "read" : level;
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
  const readIds = levels
    .filter((item) => RANK[item.level] >= RANK.read)
    .map((item) => item.id);
  const writeIds = levels
    .filter((item) => item.level === "write")
    .map((item) => item.id);
  const allowAll =
    defaultLevel !== "off" && levels.every((item) => item.level !== "off");
  const writeAll =
    defaultLevel === "write" && levels.every((item) => item.level === "write");
  return {
    allowAll,
    readIds,
    writeMode: writeAll ? "all" : writeIds.length > 0 ? "selected" : "none",
    writeIds,
  };
}
