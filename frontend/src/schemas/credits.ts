import { z } from "zod";

/** Exact API amounts never pass through JavaScript's floating-point parser. */
export const creditsSchema = z.string().regex(/^-?\d+(?:\.\d{1,12})?$/);
