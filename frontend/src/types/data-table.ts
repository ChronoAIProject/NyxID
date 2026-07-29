export interface DataTableSearchField<Key extends string> {
  readonly key: Key;
  readonly label: string;
}

export interface DataTableSearchGroup<Key extends string> {
  readonly field: Key;
  readonly values: readonly string[];
}

/**
 * `text` is a free-text-only filter: the column it targets is too
 * high-cardinality to enumerate as checkboxes (UUIDs, IPs, User-Agent strings),
 * so it carries no options and is matched server-side as a `contains`.
 */
export type DataTableFilterValueType = "enum" | "boolean" | "date" | "text";
export type DataTableFilterOperator =
  | "is"
  | "includes"
  | "between"
  | "contains";

export interface DataTableFilterOption {
  readonly value: string;
  readonly label: string;
}

export interface DataTableFilterField<Key extends string> {
  readonly key: Key;
  readonly label: string;
  readonly value_type: DataTableFilterValueType;
  readonly operator: DataTableFilterOperator;
  readonly multiple?: boolean;
  readonly options: readonly DataTableFilterOption[];
  readonly max_values?: number;
  readonly date_modes?: readonly ("dates" | "range")[];
  /**
   * Whether the filter also takes free text, matched server-side as a
   * `contains` and OR'd with whichever options are checked.
   */
  readonly supports_custom_text?: boolean;
}

export type DataTableFilterSelections<Key extends string> = Partial<
  Record<Key, readonly string[]>
>;

export interface AppliedDataTableFilter<Key extends string> {
  readonly field: DataTableFilterField<Key>;
  readonly values: readonly string[];
  readonly valueLabels: readonly string[];
  readonly operatorLabel?: string;
  readonly valueSummary?: string;
  /** Set on the chip carrying a filter's custom text, not its checked options. */
  readonly custom?: boolean;
}

export type DataTableSearchApplyMode = "submit" | "blur";
