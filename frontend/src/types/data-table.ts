export interface DataTableSearchField<Key extends string> {
  readonly key: Key;
  readonly label: string;
}

export interface DataTableSearchGroup<Key extends string> {
  readonly field: Key;
  readonly values: readonly string[];
}

export type DataTableFilterValueType = "enum" | "boolean" | "date";
export type DataTableFilterOperator = "is" | "includes" | "between";

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
}

export type DataTableSearchApplyMode = "submit" | "blur";
