import { z } from "zod";

export const assistantOneTimeMaterialSchema = z
  .enum(["delivered", "unavailable"])
  .optional();
