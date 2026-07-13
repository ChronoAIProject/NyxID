import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef, useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type {
  AppliedDataTableFilter,
  DataTableFilterField,
  DataTableFilterSelections,
  DataTableSearchApplyMode,
  DataTableSearchField,
} from "@/types/data-table";
import {
  DataTableFilterChips,
  DataTableFilterPopover,
  DataTableSearch,
} from "./data-table-controls";

vi.mock("@/components/ui/date-picker", () => ({
  DatePicker: (
    props:
      | {
          readonly mode: "multiple";
          readonly values: readonly string[];
          readonly maxSelections?: number;
          readonly onValuesChange: (values: readonly string[]) => void;
          readonly ariaLabel?: string;
        }
      | {
          readonly mode?: "single";
          readonly value: string | null;
          readonly onChange: (value: string | null) => void;
          readonly ariaLabel?: string;
          readonly minDate?: string;
        },
  ) => {
    if (props.mode === "multiple") {
      return (
        <input
          type="date"
          aria-label={props.ariaLabel}
          data-max-selections={props.maxSelections}
          value=""
          onChange={(event) => {
            const value = event.target.value;
            if (value && !props.values.includes(value)) {
              props.onValuesChange([...props.values, value].sort());
            }
          }}
        />
      );
    }
    return (
      <input
        type="date"
        aria-label={props.ariaLabel}
        value={props.value ?? ""}
        min={props.minDate}
        onChange={(event) => props.onChange(event.target.value || null)}
      />
    );
  },
}));

type SearchKey = "actor" | "resource";
type FilterKey = "severity" | "review_state" | "occurred_at" | "retained_until";

const SEARCH_FIELDS: readonly DataTableSearchField<SearchKey>[] = [
  { key: "actor", label: "Actor" },
  { key: "resource", label: "Resource" },
];

const SEVERITY_FIELD: DataTableFilterField<FilterKey> = {
  key: "severity",
  label: "Severity",
  value_type: "enum",
  operator: "is",
  multiple: true,
  options: [
    { value: "info", label: "Informational" },
    { value: "warning", label: "Warning" },
    { value: "critical", label: "Critical" },
  ],
};

const CUSTOM_TEXT_FIELD: DataTableFilterField<FilterKey> = {
  ...SEVERITY_FIELD,
  supports_custom_text: true,
};

const REVIEW_STATE_FIELD: DataTableFilterField<FilterKey> = {
  key: "review_state",
  label: "Review state",
  value_type: "boolean",
  operator: "is",
  multiple: false,
  options: [
    { value: "reviewed", label: "Reviewed" },
    { value: "unreviewed", label: "Unreviewed" },
  ],
};

const DATE_FIELD: DataTableFilterField<FilterKey> = {
  key: "occurred_at",
  label: "Occurred",
  value_type: "date",
  operator: "between",
  multiple: true,
  options: [],
  max_values: 2,
  date_modes: ["dates", "range"],
};

const RANGE_ONLY_DATE_FIELD: DataTableFilterField<FilterKey> = {
  ...DATE_FIELD,
  date_modes: ["range"],
};

const DATES_ONLY_DATE_FIELD: DataTableFilterField<FilterKey> = {
  key: "retained_until",
  label: "Retained until",
  value_type: "date",
  operator: "is",
  multiple: true,
  options: [],
  date_modes: ["dates"],
};

function SearchHarness({
  initialValue = "",
  initialField = null,
  onApply,
  onCancel = () => undefined,
}: {
  readonly initialValue?: string;
  readonly initialField?: SearchKey | null;
  readonly onApply: (mode: DataTableSearchApplyMode) => void;
  readonly onCancel?: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const [selectedField, setSelectedField] = useState<SearchKey | null>(
    initialField,
  );
  const inputRef = useRef<HTMLInputElement>(null);

  return (
    <div>
      <DataTableSearch
        fields={SEARCH_FIELDS}
        value={value}
        selectedField={selectedField}
        inputRef={inputRef}
        ariaLabel="Search audit events"
        allFieldsLabel="Everything"
        onValueChange={setValue}
        onFieldChange={setSelectedField}
        onApply={onApply}
        onCancel={onCancel}
      />
      <button type="button">Outside control</button>
    </div>
  );
}

function FilterHarness({
  fields,
  initialKey,
  values = {},
  customValues = {},
  onApply,
}: {
  readonly fields: readonly DataTableFilterField<FilterKey>[];
  readonly initialKey: FilterKey;
  readonly values?: DataTableFilterSelections<FilterKey>;
  readonly customValues?: DataTableFilterSelections<FilterKey>;
  readonly onApply: (
    selections: DataTableFilterSelections<FilterKey>,
    customSelections: DataTableFilterSelections<FilterKey>,
  ) => void;
}) {
  const [selectedKey, setSelectedKey] = useState(initialKey);
  return (
    <DataTableFilterPopover
      fields={fields}
      values={values}
      customValues={customValues}
      open
      selectedKey={selectedKey}
      activeCount={0}
      onOpenChange={() => undefined}
      onSelectField={setSelectedKey}
      onApply={onApply}
    />
  );
}

describe("DataTableSearch", () => {
  it("uses configurable labels and waits for Enter before applying typed text", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(<SearchHarness onApply={onApply} />);

    const input = screen.getByRole("textbox", {
      name: "Search audit events",
    });
    expect(input).toHaveAttribute("placeholder", "Search everything");

    await user.type(input, "failed login");
    expect(onApply).not.toHaveBeenCalled();

    await user.keyboard("{Enter}");
    expect(onApply).toHaveBeenCalledOnce();
    expect(onApply).toHaveBeenCalledWith("submit");
  });

  it("applies on full-control blur but not when focus moves to its field selector", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    const firstRender = render(
      <SearchHarness initialValue="billing" onApply={onApply} />,
    );

    const input = screen.getByRole("textbox", {
      name: "Search audit events",
    });
    const fieldSelector = screen.getByRole("combobox", {
      name: "Search field",
    });
    await user.click(input);
    await user.click(fieldSelector);
    expect(onApply).not.toHaveBeenCalled();

    firstRender.unmount();
    render(<SearchHarness initialValue="billing" onApply={onApply} />);
    await user.click(
      screen.getByRole("textbox", { name: "Search audit events" }),
    );
    await user.click(screen.getByRole("button", { name: "Outside control" }));
    expect(onApply).toHaveBeenCalledWith("blur");
  });

  it("selects from configured fields and cancels the draft with Escape", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    render(
      <SearchHarness
        initialValue="ledger"
        onApply={() => undefined}
        onCancel={onCancel}
      />,
    );

    await user.click(
      screen.getByRole("combobox", {
        name: "Search field",
      }),
    );
    await user.click(screen.getByRole("option", { name: "Resource" }));
    expect(
      screen.getByRole("textbox", { name: "Search audit events" }),
    ).toHaveAttribute("placeholder", "Search resource");

    await user.type(
      screen.getByRole("textbox", { name: "Search audit events" }),
      "{Escape}",
    );
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe("DataTableFilterPopover", () => {
  it("replaces an open draft when controlled filter values change", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    const { rerender } = render(
      <FilterHarness
        fields={[SEVERITY_FIELD]}
        initialKey="severity"
        values={{ severity: ["info"] }}
        onApply={onApply}
      />,
    );

    expect(
      screen.getByRole("checkbox", { name: "Informational" }),
    ).toBeChecked();
    expect(
      screen.getByRole("checkbox", { name: "Critical" }),
    ).not.toBeChecked();

    rerender(
      <FilterHarness
        fields={[SEVERITY_FIELD]}
        initialKey="severity"
        values={{ severity: ["critical"] }}
        onApply={onApply}
      />,
    );

    expect(
      screen.getByRole("checkbox", { name: "Informational" }),
    ).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: "Critical" })).toBeChecked();

    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenCalledWith({ severity: ["critical"] }, {});
  });

  it("applies custom text alongside the checked options, only where the field allows it", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[CUSTOM_TEXT_FIELD, REVIEW_STATE_FIELD]}
        initialKey="severity"
        values={{ severity: ["info"] }}
        onApply={onApply}
      />,
    );

    const input = screen.getByRole("textbox", { name: "Custom Severity value" });
    const add = screen.getByRole("button", { name: "Add" });
    expect(add).toBeDisabled();

    // Whitespace-only text is not a value, and neither is a duplicate.
    await user.type(input, "   ");
    expect(add).toBeDisabled();
    await user.clear(input);

    await user.type(input, "  acme  ");
    await user.click(add);
    expect(input).toHaveValue("");
    await user.type(input, "ACME");
    expect(add).toBeDisabled();
    await user.clear(input);

    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenCalledWith(
      { severity: ["info"] },
      { severity: ["acme"] },
    );

    // A field that does not declare custom text gets no input at all.
    await user.click(screen.getByRole("button", { name: /Review state/ }));
    expect(
      screen.queryByRole("textbox", { name: /Custom .* value/ }),
    ).not.toBeInTheDocument();
  });

  it("removes a custom value and counts it toward the field's selection count", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[CUSTOM_TEXT_FIELD]}
        initialKey="severity"
        values={{ severity: ["info"] }}
        customValues={{ severity: ["acme", "widgets"] }}
        onApply={onApply}
      />,
    );

    expect(screen.getByText("3 selected")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Remove custom Severity value acme" }),
    );
    expect(screen.getByText("2 selected")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenCalledWith(
      { severity: ["info"] },
      { severity: ["widgets"] },
    );
  });

  it("clears a field's options and its custom text together", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[CUSTOM_TEXT_FIELD]}
        initialKey="severity"
        values={{ severity: ["info"] }}
        customValues={{ severity: ["acme"] }}
        onApply={onApply}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Clear all" }));
    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenCalledWith({ severity: [] }, { severity: [] });
  });

  it("treats Select all as a distinct tri-state option and conditionally shows Clear all", async () => {
    const user = userEvent.setup();
    render(
      <FilterHarness
        fields={[SEVERITY_FIELD]}
        initialKey="severity"
        onApply={() => undefined}
      />,
    );

    const selectAll = screen.getByRole("checkbox", {
      name: "Select all 3 values",
    });
    expect(selectAll).toHaveAttribute("aria-checked", "false");
    expect(screen.queryByRole("button", { name: "Clear all" })).toBeNull();

    await user.click(screen.getByRole("checkbox", { name: "Warning" }));
    expect(selectAll).toHaveAttribute("aria-checked", "mixed");
    expect(screen.getByRole("button", { name: "Clear all" })).toBeVisible();

    await user.click(selectAll);
    expect(selectAll).toHaveAttribute("aria-checked", "true");

    await user.click(selectAll);
    expect(selectAll).toHaveAttribute("aria-checked", "false");
    expect(screen.queryByRole("button", { name: "Clear all" })).toBeNull();
  });

  it("keeps a single-select field to one selected value", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[REVIEW_STATE_FIELD]}
        initialKey="review_state"
        onApply={onApply}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Reviewed" }));
    await user.click(screen.getByRole("button", { name: "Unreviewed" }));
    expect(screen.getByRole("button", { name: "Reviewed" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("button", { name: "Unreviewed" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenCalledWith(
      {
        review_state: ["unreviewed"],
      },
      {},
    );
  });

  it("uses configured date labels and limits while applying multiple exact dates", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[DATE_FIELD]}
        initialKey="occurred_at"
        onApply={onApply}
      />,
    );

    expect(screen.queryByRole("checkbox", { name: /Select all/ })).toBeNull();
    const picker = screen.getByLabelText("Occurred on selected dates");
    expect(picker).toHaveAttribute("data-max-selections", "2");
    fireEvent.change(picker, { target: { value: "2026-07-03" } });
    fireEvent.change(picker, { target: { value: "2026-07-08" } });
    expect(screen.getByText("2 selected")).toBeVisible();

    await user.click(
      screen.getByRole("button", {
        name: "Remove selected date 2026-07-03",
      }),
    );
    expect(screen.getByText("1 selected")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear all" }));
    expect(screen.getByText("0 selected")).toBeVisible();

    fireEvent.change(picker, { target: { value: "2026-07-03" } });
    fireEvent.change(picker, { target: { value: "2026-07-08" } });
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(onApply).toHaveBeenCalledWith(
      {
        occurred_at: ["dates", "2026-07-03", "2026-07-08"],
      },
      {},
    );
  });

  it("supports an inclusive date range and blocks an inverted range", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();
    render(
      <FilterHarness
        fields={[DATE_FIELD]}
        initialKey="occurred_at"
        onApply={onApply}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Date range" }));
    fireEvent.change(screen.getByLabelText("Occurred from date"), {
      target: { value: "2026-07-01" },
    });
    fireEvent.change(screen.getByLabelText("Occurred to date"), {
      target: { value: "2026-07-31" },
    });
    await user.click(screen.getByRole("button", { name: "Apply filters" }));
    expect(onApply).toHaveBeenLastCalledWith(
      {
        occurred_at: ["range", "2026-07-01", "2026-07-31"],
      },
      {},
    );

    fireEvent.change(screen.getByLabelText("Occurred from date"), {
      target: { value: "2026-08-01" },
    });
    expect(
      screen.getByText("The end date must be on or after the start date."),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Apply filters" }),
    ).toBeDisabled();
  });

  it("keeps date mode UI isolated when switching between date fields", async () => {
    const user = userEvent.setup();
    render(
      <FilterHarness
        fields={[RANGE_ONLY_DATE_FIELD, DATES_ONLY_DATE_FIELD]}
        initialKey="occurred_at"
        values={{
          occurred_at: ["range", "2026-07-01", "2026-07-31"],
          retained_until: ["dates", "2026-08-15"],
        }}
        onApply={() => undefined}
      />,
    );

    expect(screen.getByLabelText("Occurred from date")).toHaveValue(
      "2026-07-01",
    );
    expect(screen.queryByLabelText("Occurred on selected dates")).toBeNull();

    await user.click(screen.getByRole("button", { name: /Retained until/ }));
    expect(
      screen.getByLabelText("Retained until on selected dates"),
    ).toBeInTheDocument();
    expect(screen.getByText("1 selected")).toBeVisible();
    expect(screen.queryByLabelText("Retained until from date")).toBeNull();

    await user.click(screen.getByRole("button", { name: /Occurred/ }));
    expect(screen.getByLabelText("Occurred from date")).toHaveValue(
      "2026-07-01",
    );
    expect(screen.queryByLabelText("Occurred on selected dates")).toBeNull();
  });
});

describe("DataTableFilterChips", () => {
  it("renders global search and configured filters without AND separators", () => {
    const filters: readonly AppliedDataTableFilter<FilterKey>[] = [
      {
        field: SEVERITY_FIELD,
        values: ["critical"],
        valueLabels: ["Critical"],
      },
      {
        field: DATE_FIELD,
        values: ["2026-07-03", "2026-07-03"],
        valueLabels: ["Jul 3, 2026"],
        operatorLabel: "is on",
        valueSummary: "Jul 3, 2026",
      },
    ];
    render(
      <DataTableFilterChips
        search="alice@example.com"
        searchFields={SEARCH_FIELDS}
        searchFilters={[]}
        filters={filters}
        onEditSearch={() => undefined}
        onRemoveSearch={() => undefined}
        onEditSearchValue={() => undefined}
        onRemoveSearchValue={() => undefined}
        onEdit={() => undefined}
        onRemove={() => undefined}
        onClear={() => undefined}
      />,
    );

    expect(screen.queryByText("AND")).not.toBeInTheDocument();
    expect(screen.getByText('"alice@example.com"')).toBeVisible();
    expect(screen.getByText("Jul 3, 2026")).toBeVisible();
  });

  it("shows OR within one scoped field without inter-chip operators", () => {
    render(
      <DataTableFilterChips
        search=""
        searchFields={SEARCH_FIELDS}
        searchFilters={[
          { field: "actor", values: ["alice", "bob"] },
          { field: "resource", values: ["billing"] },
        ]}
        filters={[]}
        onEditSearch={() => undefined}
        onRemoveSearch={() => undefined}
        onEditSearchValue={() => undefined}
        onRemoveSearchValue={() => undefined}
        onEdit={() => undefined}
        onRemove={() => undefined}
        onClear={() => undefined}
      />,
    );

    expect(screen.getAllByText("OR")).toHaveLength(1);
    expect(screen.queryByText("AND")).not.toBeInTheDocument();
    expect(
      screen.getByRole("group", {
        name: "Actor search, matches any term",
      }),
    ).toHaveTextContent(/Actor contains.*"alice".*OR.*"bob"/);
  });
});
