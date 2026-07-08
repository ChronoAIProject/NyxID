import { describe, expect, it } from "vitest";
import {
  firstNestedErrorMessage,
  flattenRowErrors,
  flattenRowFieldErrors,
  zodIssuesToRowFieldErrors,
} from "./form-errors";

describe("firstNestedErrorMessage", () => {
  it("returns a direct message", () => {
    expect(firstNestedErrorMessage({ message: "Required" })).toBe("Required");
  });

  it("finds a deeply nested message", () => {
    const error = {
      trigger: {
        json_field_equals: {
          path: { type: "too_small", message: "JSON path is required" },
        },
      },
    };
    expect(firstNestedErrorMessage(error)).toBe("JSON path is required");
  });

  it("does not traverse ref/type/types keys", () => {
    const error = {
      type: "custom",
      types: { custom: { message: "from types" } },
      ref: { message: "from a DOM ref" },
    };
    expect(firstNestedErrorMessage(error)).toBeUndefined();
  });

  it("returns undefined for empty or non-object input", () => {
    expect(firstNestedErrorMessage(undefined)).toBeUndefined();
    expect(firstNestedErrorMessage("nope")).toBeUndefined();
    expect(firstNestedErrorMessage({})).toBeUndefined();
  });
});

describe("flattenRowErrors", () => {
  it("maps numeric-keyed nested errors to row messages", () => {
    const error = {
      0: { weight: { message: "Weight must be at most 1000" } },
      2: { name: { message: "Header name is required" } },
      message: "array-root message ignored for rows",
    };
    expect(flattenRowErrors(error)).toEqual({
      0: "Weight must be at most 1000",
      2: "Header name is required",
    });
  });

  it("returns an empty map when there is no error", () => {
    expect(flattenRowErrors(undefined)).toEqual({});
  });
});

describe("flattenRowFieldErrors", () => {
  it("keeps per-field messages within each row", () => {
    const error = {
      0: {
        name: { type: "too_small", message: "Header name is required" },
        value: { type: "too_small", message: "Header value is required" },
      },
      2: { value: { message: "Header value must not be blank" } },
    };
    expect(flattenRowFieldErrors(error)).toEqual({
      0: {
        name: "Header name is required",
        value: "Header value is required",
      },
      2: { value: "Header value must not be blank" },
    });
  });

  it("falls back to a row-level root message", () => {
    const error = { 1: { type: "custom", message: "Row is invalid" } };
    expect(flattenRowFieldErrors(error)).toEqual({
      1: { root: "Row is invalid" },
    });
  });

  it("returns an empty map when there is no error", () => {
    expect(flattenRowFieldErrors(undefined)).toEqual({});
  });
});

describe("zodIssuesToRowFieldErrors", () => {
  it("maps issue paths to row/field messages, keeping the first per field", () => {
    const issues = [
      { path: [0, "value"], message: "Header value is required" },
      { path: [0, "value"], message: "Header value must not be blank" },
      { path: [2, "name"], message: "Header name is required" },
      { path: [1], message: "Row-level issue" },
      { path: ["not-a-row"], message: "List-level issue ignored" },
    ];
    expect(zodIssuesToRowFieldErrors(issues)).toEqual({
      0: { value: "Header value is required" },
      1: { root: "Row-level issue" },
      2: { name: "Header name is required" },
    });
  });

  it("handles deep paths like trigger.json_field_equals.path", () => {
    const issues = [
      {
        path: [1, "trigger", "json_field_equals", "path"],
        message: "JSON path is required",
      },
    ];
    expect(zodIssuesToRowFieldErrors(issues)).toEqual({
      1: { trigger: "JSON path is required" },
    });
  });
});
