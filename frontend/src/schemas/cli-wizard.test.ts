import { describe, expect, it } from "vitest"
import {
  apiKeyNameSchema,
  firstError,
  nodeNameSchema,
  platformSchema,
  serviceLabelSchema,
  serviceSlugSchema,
} from "./cli-wizard"

describe("cli-wizard schemas", () => {
  describe("nodeNameSchema", () => {
    it("accepts lowercase kebab-case", () => {
      expect(nodeNameSchema.safeParse("edge-tokyo").success).toBe(true)
    })

    it("accepts digits and hyphens", () => {
      expect(nodeNameSchema.safeParse("node-42").success).toBe(true)
    })

    it("rejects spaces (the bug we hit live)", () => {
      const r = nodeNameSchema.safeParse("ccmini test")
      expect(r.success).toBe(false)
      if (!r.success) {
        expect(r.error.issues[0]?.message).toMatch(/hyphen|letter|digit/i)
      }
    })

    it("rejects uppercase", () => {
      expect(nodeNameSchema.safeParse("EdgeTokyo").success).toBe(false)
    })

    it("rejects empty string", () => {
      expect(nodeNameSchema.safeParse("").success).toBe(false)
    })

    it("rejects names longer than 64 chars", () => {
      expect(nodeNameSchema.safeParse("a".repeat(65)).success).toBe(false)
    })

    it("accepts names exactly 64 chars", () => {
      expect(nodeNameSchema.safeParse("a".repeat(64)).success).toBe(true)
    })
  })

  describe("serviceSlugSchema", () => {
    it("accepts kebab-case", () => {
      expect(serviceSlugSchema.safeParse("my-service").success).toBe(true)
    })

    it("rejects leading hyphen", () => {
      const r = serviceSlugSchema.safeParse("-foo")
      expect(r.success).toBe(false)
      if (!r.success) {
        expect(r.error.issues[0]?.message).toMatch(/start|end|hyphen/i)
      }
    })

    it("rejects trailing hyphen", () => {
      expect(serviceSlugSchema.safeParse("foo-").success).toBe(false)
    })

    it("rejects uppercase", () => {
      expect(serviceSlugSchema.safeParse("Foo").success).toBe(false)
    })
  })

  describe("apiKeyNameSchema", () => {
    it("allows any printable characters", () => {
      expect(apiKeyNameSchema.safeParse("Coding Agent #1 (prod)").success).toBe(true)
    })

    it("caps at 200 chars", () => {
      expect(apiKeyNameSchema.safeParse("a".repeat(201)).success).toBe(false)
      expect(apiKeyNameSchema.safeParse("a".repeat(200)).success).toBe(true)
    })

    it("rejects empty", () => {
      expect(apiKeyNameSchema.safeParse("").success).toBe(false)
    })
  })

  describe("serviceLabelSchema", () => {
    it("allows free text", () => {
      expect(serviceLabelSchema.safeParse("Prod OpenAI key").success).toBe(true)
    })

    it("rejects empty", () => {
      expect(serviceLabelSchema.safeParse("").success).toBe(false)
    })
  })

  describe("platformSchema", () => {
    it("accepts known platforms", () => {
      expect(platformSchema.safeParse("claude-code").success).toBe(true)
      expect(platformSchema.safeParse("codex").success).toBe(true)
    })

    it("accepts empty string", () => {
      expect(platformSchema.safeParse("").success).toBe(true)
    })

    it("rejects unknown platforms", () => {
      expect(platformSchema.safeParse("bogus").success).toBe(false)
    })
  })

  describe("firstError", () => {
    it("returns null for valid values", () => {
      expect(firstError(nodeNameSchema, "edge-tokyo")).toBeNull()
    })

    it("returns the first error message for invalid values", () => {
      const msg = firstError(nodeNameSchema, "invalid name")
      expect(msg).not.toBeNull()
      expect(typeof msg).toBe("string")
    })
  })
})
