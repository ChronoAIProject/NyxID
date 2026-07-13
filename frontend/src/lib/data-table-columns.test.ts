import { describe, expect, it } from "vitest";
import {
  clampColumnWidth,
  columnWidthVar,
  columnWidthVars,
  frozenColumnFields,
  stickyColumnLeft,
  sumOfColumnWidths,
} from "./data-table-columns";

const ORDER = ["client_name", "client_type", "created_by", "broker"] as const;

describe("column width variables", () => {
  it("names one variable per column and sums them in order", () => {
    expect(columnWidthVar("client_name")).toBe("--col-w-client_name");
    expect(sumOfColumnWidths(["client_name", "client_type"])).toBe(
      "calc(var(--col-w-client_name) + var(--col-w-client_type))",
    );
    expect(
      columnWidthVars(["client_name", "client_type"], {
        client_name: 260,
        client_type: 140,
      }),
    ).toEqual({
      "--col-w-client_name": "260px",
      "--col-w-client_type": "140px",
    });
  });

  it("clamps a width to the allowed range and rounds sub-pixel drags", () => {
    expect(clampColumnWidth(260.4, 96, 640)).toBe(260);
    expect(clampColumnWidth(-40, 96, 640)).toBe(96);
    expect(clampColumnWidth(2000, 96, 640)).toBe(640);
  });
});

describe("frozen columns", () => {
  it("freezes every column through the pinned one", () => {
    expect(frozenColumnFields(ORDER, "created_by")).toEqual([
      "client_name",
      "client_type",
      "created_by",
    ]);
    expect(frozenColumnFields(ORDER, null)).toEqual([]);
  });

  it("offsets each frozen column by the widths of the frozen columns before it", () => {
    const frozen = frozenColumnFields(ORDER, "created_by");

    // Offsets resolve the same variables a resize rewrites, so a frozen column
    // stays pinned to its neighbour's edge while the drag is in flight.
    expect(stickyColumnLeft(frozen, "client_name")).toBe("0px");
    expect(stickyColumnLeft(frozen, "client_type")).toBe(
      "calc(var(--col-w-client_name))",
    );
    expect(stickyColumnLeft(frozen, "created_by")).toBe(
      "calc(var(--col-w-client_name) + var(--col-w-client_type))",
    );
  });

  it("gives no offset to a column that is not frozen", () => {
    const frozen = frozenColumnFields(ORDER, "client_type");
    expect(stickyColumnLeft(frozen, "broker")).toBeUndefined();
  });

  it("reorders offsets with the columns", () => {
    const reordered = ["client_type", "client_name", "created_by"] as const;
    const frozen = frozenColumnFields(reordered, "created_by");

    expect(stickyColumnLeft(frozen, "client_type")).toBe("0px");
    expect(stickyColumnLeft(frozen, "client_name")).toBe(
      "calc(var(--col-w-client_type))",
    );
    expect(stickyColumnLeft(frozen, "created_by")).toBe(
      "calc(var(--col-w-client_type) + var(--col-w-client_name))",
    );
  });
});
