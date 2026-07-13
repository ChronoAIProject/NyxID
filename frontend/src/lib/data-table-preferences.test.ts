import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  isDefaultColumnLayout,
  loadColumnPreferences,
  sanitizeColumnPreferences,
  saveColumnPreferences,
  type DataTableColumnPreferences,
} from "./data-table-preferences";

type Field = "name" | "type" | "created";

const BOUNDS = { min: 96, max: 640 };

const DEFAULTS: DataTableColumnPreferences<Field> = {
  order: ["name", "type", "created"],
  frozenThrough: null,
  widths: { name: 260, type: 140, created: 190 },
};

const KEY = "nyxid.table.test.columns.v1";

beforeEach(() => {
  localStorage.clear();
});

describe("sanitizeColumnPreferences", () => {
  it("keeps a valid stored layout", () => {
    expect(
      sanitizeColumnPreferences(
        {
          order: ["type", "name", "created"],
          frozenThrough: "name",
          widths: { name: 320, type: 140, created: 190 },
        },
        DEFAULTS,
        BOUNDS,
      ),
    ).toEqual({
      order: ["type", "name", "created"],
      frozenThrough: "name",
      widths: { name: 320, type: 140, created: 190 },
    });
  });

  it("falls back to the defaults for anything that is not a layout", () => {
    for (const raw of [null, undefined, 7, "{}", [], true]) {
      expect(sanitizeColumnPreferences(raw, DEFAULTS, BOUNDS)).toEqual(
        DEFAULTS,
      );
    }
  });

  it("drops columns it no longer knows and re-adds ones it never saw", () => {
    // Written by an older build: `type` did not exist yet, `legacy` since gone.
    const sanitized = sanitizeColumnPreferences(
      { order: ["created", "legacy", "name"], frozenThrough: "legacy" },
      DEFAULTS,
      BOUNDS,
    );

    expect(sanitized.order).toEqual(["created", "name", "type"]);
    // A freeze point that survived only as a dropped column must not stick.
    expect(sanitized.frozenThrough).toBeNull();
  });

  it("ignores duplicate columns in a stored order", () => {
    expect(
      sanitizeColumnPreferences(
        { order: ["name", "name", "type"] },
        DEFAULTS,
        BOUNDS,
      ).order,
    ).toEqual(["name", "type", "created"]);
  });

  it("rejects widths outside the current bounds instead of clamping them", () => {
    const sanitized = sanitizeColumnPreferences(
      { widths: { name: 5000, type: 1, created: 240.6 } },
      DEFAULTS,
      BOUNDS,
    );

    // Out-of-bounds values came from different rules, so the default is the
    // honest answer -- clamping would silently invent a width the user never set.
    expect(sanitized.widths).toEqual({ name: 260, type: 140, created: 241 });
  });

  it("ignores widths that are not finite numbers", () => {
    expect(
      sanitizeColumnPreferences(
        { widths: { name: "320", type: Number.NaN, created: null } },
        DEFAULTS,
        BOUNDS,
      ).widths,
    ).toEqual(DEFAULTS.widths);
  });
});

describe("load / save", () => {
  it("round-trips a layout through storage", () => {
    const layout: DataTableColumnPreferences<Field> = {
      order: ["type", "name", "created"],
      frozenThrough: "type",
      widths: { name: 300, type: 140, created: 190 },
    };

    saveColumnPreferences(KEY, layout, DEFAULTS);
    expect(loadColumnPreferences(KEY, DEFAULTS, BOUNDS)).toEqual(layout);
  });

  it("clears the row once the table is back to its defaults", () => {
    saveColumnPreferences(
      KEY,
      { ...DEFAULTS, frozenThrough: "name" },
      DEFAULTS,
    );
    expect(localStorage.getItem(KEY)).not.toBeNull();

    saveColumnPreferences(KEY, DEFAULTS, DEFAULTS);
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(loadColumnPreferences(KEY, DEFAULTS, BOUNDS)).toEqual(DEFAULTS);
  });

  it("returns the defaults for a corrupt row rather than throwing", () => {
    localStorage.setItem(KEY, "{not json");
    expect(loadColumnPreferences(KEY, DEFAULTS, BOUNDS)).toEqual(DEFAULTS);
  });

  it("survives storage being unavailable", () => {
    const getItem = vi
      .spyOn(Storage.prototype, "getItem")
      .mockImplementation(() => {
        throw new Error("SecurityError");
      });
    const setItem = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new Error("QuotaExceededError");
      });

    expect(loadColumnPreferences(KEY, DEFAULTS, BOUNDS)).toEqual(DEFAULTS);
    expect(() => {
      saveColumnPreferences(
        KEY,
        { ...DEFAULTS, frozenThrough: "name" },
        DEFAULTS,
      );
    }).not.toThrow();

    getItem.mockRestore();
    setItem.mockRestore();
  });
});

describe("isDefaultColumnLayout", () => {
  it("detects each way a layout can differ", () => {
    expect(isDefaultColumnLayout(DEFAULTS, DEFAULTS)).toBe(true);
    expect(
      isDefaultColumnLayout({ ...DEFAULTS, frozenThrough: "name" }, DEFAULTS),
    ).toBe(false);
    expect(
      isDefaultColumnLayout(
        { ...DEFAULTS, order: ["type", "name", "created"] },
        DEFAULTS,
      ),
    ).toBe(false);
    expect(
      isDefaultColumnLayout(
        { ...DEFAULTS, widths: { ...DEFAULTS.widths, name: 300 } },
        DEFAULTS,
      ),
    ).toBe(false);
  });
});
