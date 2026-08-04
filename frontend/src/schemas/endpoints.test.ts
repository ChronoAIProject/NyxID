import { describe, it, expect } from "vitest";
import {
  createEndpointSchema,
  updateEndpointSchema,
  ENDPOINT_METHODS,
} from "./endpoints";

describe("ENDPOINT_METHODS", () => {
  it("contains standard HTTP methods", () => {
    expect(ENDPOINT_METHODS).toEqual(["GET", "POST", "PUT", "DELETE", "PATCH"]);
  });
});

describe("createEndpointSchema", () => {
  const validData = {
    name: "get_users",
    method: "GET" as const,
    path: "/users",
  };

  it("accepts valid endpoint data", () => {
    const result = createEndpointSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("accepts endpoint with all optional fields", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      description: "Get all users",
      parameters: '{"limit": "number"}',
      request_body_schema: '{"name": "string"}',
      response_description: "List of users",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty name", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      name: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects name starting with uppercase", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      name: "GetUsers",
    });
    expect(result.success).toBe(false);
  });

  it("rejects name starting with a number", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      name: "1get_users",
    });
    expect(result.success).toBe(false);
  });

  it("accepts name with underscores and digits", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      name: "get_users_v2",
    });
    expect(result.success).toBe(true);
  });

  it("rejects name over 100 characters", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      name: "a".repeat(101),
    });
    expect(result.success).toBe(false);
  });

  it("rejects path not starting with /", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      path: "users",
    });
    expect(result.success).toBe(false);
  });

  it("rejects empty path", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      path: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects invalid JSON in parameters", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      parameters: "not valid json",
    });
    expect(result.success).toBe(false);
  });

  it("accepts empty string for parameters", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      parameters: "",
    });
    expect(result.success).toBe(true);
  });

  it("rejects invalid JSON in request_body_schema", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      request_body_schema: "{bad json",
    });
    expect(result.success).toBe(false);
  });

  it("rejects description over 500 characters", () => {
    const result = createEndpointSchema.safeParse({
      ...validData,
      description: "a".repeat(501),
    });
    expect(result.success).toBe(false);
  });
});

describe("updateEndpointSchema", () => {
  it("accepts minimal update data", () => {
    const result = updateEndpointSchema.safeParse({});
    expect(result.success).toBe(true);
  });

  it("accepts partial update with is_active", () => {
    const result = updateEndpointSchema.safeParse({
      is_active: false,
    });
    expect(result.success).toBe(true);
  });

  it("accepts full update data", () => {
    const result = updateEndpointSchema.safeParse({
      name: "updated_endpoint",
      method: "POST" as const,
      path: "/updated",
      description: "Updated description",
      is_active: true,
    });
    expect(result.success).toBe(true);
  });

  it("rejects invalid name format on update", () => {
    const result = updateEndpointSchema.safeParse({
      name: "Invalid Name",
    });
    expect(result.success).toBe(false);
  });
});
