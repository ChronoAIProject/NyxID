import { describe, it, expect } from "vitest";
import { updateUserSchema, createUserSchema } from "./admin";

describe("createUserSchema", () => {
  const validData = {
    email: "user@example.com",
    password: "StrongPass1",
    role: "user" as const,
  };

  it("accepts valid user data", () => {
    const result = createUserSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("accepts admin role", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      role: "admin",
    });
    expect(result.success).toBe(true);
  });

  it("accepts data with optional display_name", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      display_name: "John Doe",
    });
    expect(result.success).toBe(true);
  });

  it("accepts empty string for display_name", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      display_name: "",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty email", () => {
    const result = createUserSchema.safeParse({ ...validData, email: "" });
    expect(result.success).toBe(false);
  });

  it("rejects invalid email", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      email: "not-email",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password shorter than 8 characters", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      password: "Short1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password over 128 characters", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      password: "A".repeat(129),
    });
    expect(result.success).toBe(false);
  });

  it("rejects invalid role", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      role: "superadmin",
    });
    expect(result.success).toBe(false);
  });

  it("rejects display_name over 200 characters", () => {
    const result = createUserSchema.safeParse({
      ...validData,
      display_name: "a".repeat(201),
    });
    expect(result.success).toBe(false);
  });
});

describe("updateUserSchema", () => {
  it("accepts empty object", () => {
    const result = updateUserSchema.safeParse({});
    expect(result.success).toBe(true);
  });

  it("accepts valid display_name", () => {
    const result = updateUserSchema.safeParse({
      display_name: "New Name",
    });
    expect(result.success).toBe(true);
  });

  it("accepts valid email", () => {
    const result = updateUserSchema.safeParse({
      email: "new@example.com",
    });
    expect(result.success).toBe(true);
  });

  it("accepts empty string for email (treated as unset)", () => {
    const result = updateUserSchema.safeParse({ email: "" });
    expect(result.success).toBe(true);
  });

  it("rejects invalid email", () => {
    const result = updateUserSchema.safeParse({ email: "bad" });
    expect(result.success).toBe(false);
  });

  it("accepts valid avatar_url", () => {
    const result = updateUserSchema.safeParse({
      avatar_url: "https://example.com/avatar.png",
    });
    expect(result.success).toBe(true);
  });

  it("rejects invalid avatar_url", () => {
    const result = updateUserSchema.safeParse({
      avatar_url: "not-a-url",
    });
    expect(result.success).toBe(false);
  });

  it("rejects avatar_url over 2048 characters", () => {
    const result = updateUserSchema.safeParse({
      avatar_url: `https://example.com/${"a".repeat(2040)}`,
    });
    expect(result.success).toBe(false);
  });

  it("rejects display_name over 200 characters", () => {
    const result = updateUserSchema.safeParse({
      display_name: "a".repeat(201),
    });
    expect(result.success).toBe(false);
  });
});
