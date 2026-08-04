import { describe, it, expect } from "vitest";
import {
  updateUserSchema,
  createUserSchema,
  createInviteCodeSchema,
} from "./admin";

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

describe("createInviteCodeSchema", () => {
  it("accepts default values (10 uses, no note)", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 10 });
    expect(result.success).toBe(true);
  });

  it("accepts max_uses at lower bound", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 1 });
    expect(result.success).toBe(true);
  });

  it("accepts max_uses at upper bound", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 1000 });
    expect(result.success).toBe(true);
  });

  it("rejects max_uses below 1", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 0 });
    expect(result.success).toBe(false);
  });

  it("rejects max_uses above 1000", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 1001 });
    expect(result.success).toBe(false);
  });

  it("rejects non-integer max_uses", () => {
    const result = createInviteCodeSchema.safeParse({ max_uses: 5.5 });
    expect(result.success).toBe(false);
  });

  it("accepts a note within the length limit", () => {
    const result = createInviteCodeSchema.safeParse({
      max_uses: 10,
      note: "alice@corp",
    });
    expect(result.success).toBe(true);
  });

  it("accepts an empty-string note", () => {
    const result = createInviteCodeSchema.safeParse({
      max_uses: 10,
      note: "",
    });
    expect(result.success).toBe(true);
  });

  it("rejects a note longer than 512 characters", () => {
    const result = createInviteCodeSchema.safeParse({
      max_uses: 10,
      note: "a".repeat(513),
    });
    expect(result.success).toBe(false);
  });
});
