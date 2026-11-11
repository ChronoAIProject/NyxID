import { z } from "zod";
const slug = z
  .string()
  .min(1)
  .max(80)
  .regex(/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/);
const text = (max: number) =>
  z
    .string()
    .min(1)
    .refine(
      (v) =>
        v.trim().length > 0 &&
        new TextEncoder().encode(v).length <= max &&
        [...v].every((c) => {
          const n = c.codePointAt(0)!;
          return (
            (n >= 32 && n < 127) ||
            n >= 160 ||
            c === "\n" ||
            c === "\r" ||
            c === "\t"
          );
        }),
      `Use 1–${max} bytes of text.`,
    );
export const helpContentSchema = z
  .object({
    articles: z
      .array(
        z.object({
          id: slug,
          title: text(100),
          body: text(6000),
          tags: z.array(text(40)).max(8),
        }),
      )
      .min(1)
      .max(30),
    use_cases: z
      .array(
        z.object({
          id: slug,
          title: text(100),
          description: text(2000),
          example_prompts: z.array(text(500)).max(8),
        }),
      )
      .max(20),
  })
  .superRefine((v, ctx) => {
    const ids = [...v.articles, ...v.use_cases].map((a) => a.id);
    if (new Set(ids).size !== ids.length)
      ctx.addIssue({
        code: "custom",
        path: ["root"],
        message: "Article and use-case IDs must be unique.",
      });
    if (new TextEncoder().encode(JSON.stringify(v)).length > 16384)
      ctx.addIssue({
        code: "custom",
        path: ["root"],
        message: "The complete library must fit within 16 KiB.",
      });
  });
export const helpLibrarySchema = z.object({
  id: z.uuid(),
  name: z.string(),
  version: z.number().int().positive(),
  published_revision: z.number().int().positive(),
  draft: helpContentSchema,
  updated_at: z.string(),
});
export const helpRevisionSchema = z.object({
  revision: z.number().int().positive(),
  content: helpContentSchema,
  published_at: z.string(),
  published_by: z.string(),
});
export const helpAssignmentSchema = z.object({
  profile_id: z.uuid(),
  library_id: z.uuid(),
  version: z.number().int().nonnegative(),
});
export type HelpContent = z.infer<typeof helpContentSchema>;
export type HelpLibrary = z.infer<typeof helpLibrarySchema>;
