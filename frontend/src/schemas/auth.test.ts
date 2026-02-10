import { describe, it, expect } from "vitest";
import {
  loginSchema,
  registerSchema,
  forgotPasswordSchema,
  resetPasswordSchema,
  changePasswordSchema,
  mfaVerifySchema,
} from "./auth";

describe("loginSchema", () => {
  it("accepts valid credentials", () => {
    const result = loginSchema.safeParse({
      email: "user@example.com",
      password: "secret123",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty email", () => {
    const result = loginSchema.safeParse({ email: "", password: "secret123" });
    expect(result.success).toBe(false);
  });

  it("rejects invalid email format", () => {
    const result = loginSchema.safeParse({
      email: "not-an-email",
      password: "secret123",
    });
    expect(result.success).toBe(false);
  });

  it("rejects empty password", () => {
    const result = loginSchema.safeParse({
      email: "user@example.com",
      password: "",
    });
    expect(result.success).toBe(false);
  });
});

describe("registerSchema", () => {
  const validData = {
    name: "John Doe",
    email: "john@example.com",
    password: "Password1",
    confirmPassword: "Password1",
  };

  it("accepts valid registration data", () => {
    const result = registerSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("rejects name shorter than 2 characters", () => {
    const result = registerSchema.safeParse({ ...validData, name: "J" });
    expect(result.success).toBe(false);
  });

  it("rejects empty name", () => {
    const result = registerSchema.safeParse({ ...validData, name: "" });
    expect(result.success).toBe(false);
  });

  it("rejects invalid email", () => {
    const result = registerSchema.safeParse({
      ...validData,
      email: "bad-email",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password shorter than 8 characters", () => {
    const result = registerSchema.safeParse({
      ...validData,
      password: "Pass1",
      confirmPassword: "Pass1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password without uppercase letter", () => {
    const result = registerSchema.safeParse({
      ...validData,
      password: "password1",
      confirmPassword: "password1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password without lowercase letter", () => {
    const result = registerSchema.safeParse({
      ...validData,
      password: "PASSWORD1",
      confirmPassword: "PASSWORD1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects password without number", () => {
    const result = registerSchema.safeParse({
      ...validData,
      password: "PasswordOnly",
      confirmPassword: "PasswordOnly",
    });
    expect(result.success).toBe(false);
  });

  it("rejects mismatched passwords", () => {
    const result = registerSchema.safeParse({
      ...validData,
      confirmPassword: "Different1",
    });
    expect(result.success).toBe(false);
  });
});

describe("forgotPasswordSchema", () => {
  it("accepts valid email", () => {
    const result = forgotPasswordSchema.safeParse({
      email: "user@example.com",
    });
    expect(result.success).toBe(true);
  });

  it("rejects empty email", () => {
    const result = forgotPasswordSchema.safeParse({ email: "" });
    expect(result.success).toBe(false);
  });

  it("rejects invalid email", () => {
    const result = forgotPasswordSchema.safeParse({ email: "not-email" });
    expect(result.success).toBe(false);
  });
});

describe("resetPasswordSchema", () => {
  const validData = {
    password: "NewPass123",
    confirmPassword: "NewPass123",
  };

  it("accepts valid reset data", () => {
    const result = resetPasswordSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("rejects short password", () => {
    const result = resetPasswordSchema.safeParse({
      password: "Np1",
      confirmPassword: "Np1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects mismatched passwords", () => {
    const result = resetPasswordSchema.safeParse({
      ...validData,
      confirmPassword: "Mismatch1",
    });
    expect(result.success).toBe(false);
  });

  it("requires uppercase letter", () => {
    const result = resetPasswordSchema.safeParse({
      password: "newpass123",
      confirmPassword: "newpass123",
    });
    expect(result.success).toBe(false);
  });

  it("requires lowercase letter", () => {
    const result = resetPasswordSchema.safeParse({
      password: "NEWPASS123",
      confirmPassword: "NEWPASS123",
    });
    expect(result.success).toBe(false);
  });

  it("requires number", () => {
    const result = resetPasswordSchema.safeParse({
      password: "NewPassOnly",
      confirmPassword: "NewPassOnly",
    });
    expect(result.success).toBe(false);
  });
});

describe("changePasswordSchema", () => {
  const validData = {
    currentPassword: "OldPass1",
    newPassword: "NewPass123",
    confirmNewPassword: "NewPass123",
  };

  it("accepts valid change password data", () => {
    const result = changePasswordSchema.safeParse(validData);
    expect(result.success).toBe(true);
  });

  it("rejects empty current password", () => {
    const result = changePasswordSchema.safeParse({
      ...validData,
      currentPassword: "",
    });
    expect(result.success).toBe(false);
  });

  it("rejects mismatched new passwords", () => {
    const result = changePasswordSchema.safeParse({
      ...validData,
      confirmNewPassword: "DifferentPass1",
    });
    expect(result.success).toBe(false);
  });

  it("rejects weak new password", () => {
    const result = changePasswordSchema.safeParse({
      ...validData,
      newPassword: "weak",
      confirmNewPassword: "weak",
    });
    expect(result.success).toBe(false);
  });
});

describe("mfaVerifySchema", () => {
  it("accepts valid 6-digit code", () => {
    const result = mfaVerifySchema.safeParse({ code: "123456" });
    expect(result.success).toBe(true);
  });

  it("rejects empty code", () => {
    const result = mfaVerifySchema.safeParse({ code: "" });
    expect(result.success).toBe(false);
  });

  it("rejects code with less than 6 digits", () => {
    const result = mfaVerifySchema.safeParse({ code: "12345" });
    expect(result.success).toBe(false);
  });

  it("rejects code with more than 6 digits", () => {
    const result = mfaVerifySchema.safeParse({ code: "1234567" });
    expect(result.success).toBe(false);
  });

  it("rejects non-numeric code", () => {
    const result = mfaVerifySchema.safeParse({ code: "abcdef" });
    expect(result.success).toBe(false);
  });

  it("rejects code with mixed characters", () => {
    const result = mfaVerifySchema.safeParse({ code: "123abc" });
    expect(result.success).toBe(false);
  });
});
