import { z } from "zod";

const shortText = z.string().min(1).max(256);
const timestamp = z
  .string()
  .min(1)
  .max(64)
  .refine((value) => Number.isFinite(Date.parse(value)), {
    message: "Expected a valid timestamp",
  });
const DEVICE_APPROVAL_ORIGIN = "https://nyx.chrono-ai.fun";
const DEVICE_APPROVAL_PATH = "/login/device";

const unavailableSchema = z
  .object({ state: z.literal("unavailable") })
  .strict();
const checkingSchema = z.object({ state: z.literal("checking") }).strict();
const signedOutSchema = z.object({ state: z.literal("signed_out") }).strict();

const authorizingSchema = z
  .object({
    state: z.literal("authorizing"),
    userCode: z.string().min(4).max(32),
    verificationUrl: z.string().url().max(2048),
    expiresAt: timestamp,
  })
  .strict()
  .superRefine((value, context) => {
    const url = new URL(value.verificationUrl);
    const queryEntries = [...url.searchParams.entries()];
    const isExpectedApprovalUrl =
      url.origin === DEVICE_APPROVAL_ORIGIN &&
      url.pathname === DEVICE_APPROVAL_PATH &&
      url.username === "" &&
      url.password === "" &&
      url.hash === "" &&
      queryEntries.length === 1 &&
      queryEntries[0]?.[0] === "user_code" &&
      queryEntries[0]?.[1] === value.userCode;

    if (!isExpectedApprovalUrl) {
      context.addIssue({
        code: "custom",
        path: ["verificationUrl"],
        message: "Expected the NyxID device approval URL",
      });
    }
  });

const serviceSchema = z
  .object({
    id: shortText,
    slug: shortText,
    label: shortText,
    state: z.enum(["enabled", "disabled", "attention"]),
  })
  .strict();

const connectedSchema = z
  .object({
    state: z.literal("connected"),
    user: z
      .object({
        id: shortText,
        email: z.string().email().max(320),
        displayName: z.string().max(256).nullish(),
        avatarUrl: z.string().url().max(2048).nullish(),
      })
      .strict(),
    capabilities: z
      .object({
        enabledCount: z.number().int().nonnegative(),
        disabledCount: z.number().int().nonnegative(),
        attentionCount: z.number().int().nonnegative(),
        checkedAt: timestamp,
        services: z.array(serviceSchema),
      })
      .strict(),
  })
  .strict();

const terminalSchema = z
  .object({
    state: z.enum(["denied", "expired"]),
    message: z.string().min(1).max(500),
  })
  .strict();

const errorSchema = z
  .object({
    state: z.literal("error"),
    error: z
      .object({
        code: z.string().min(1).max(128),
        message: z.string().min(1).max(500),
        retryable: z.boolean(),
        retryAction: z
          .enum(["connect", "cancel", "refresh", "logout"])
          .nullable(),
      })
      .superRefine((error, context) => {
        if (error.retryable !== (error.retryAction !== null)) {
          context.addIssue({
            code: "custom",
            path: ["retryable"],
            message: "retryable must match retryAction",
          });
        }
      })
      .strict(),
  })
  .strict();

export const nyxIdViewSchema = z.discriminatedUnion("state", [
  unavailableSchema,
  checkingSchema,
  signedOutSchema,
  authorizingSchema,
  connectedSchema,
  terminalSchema,
  errorSchema,
]);

export type NyxIdView = z.infer<typeof nyxIdViewSchema>;

export function parseNyxIdView(value: unknown): NyxIdView {
  return nyxIdViewSchema.parse(value);
}
