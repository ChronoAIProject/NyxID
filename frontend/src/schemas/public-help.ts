import { z } from "zod";

export const publicUseCasesSchema = z.object({
  use_cases: z.array(
    z.object({
      id: z.string(),
      title: z.string(),
      description: z.string(),
      example_prompts: z.array(z.string()),
    }),
  ),
});
export type PublicUseCase = z.infer<
  typeof publicUseCasesSchema
>["use_cases"][number];
