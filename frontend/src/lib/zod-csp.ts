import { z } from "zod/v4";
export * from "zod/v4";

// Zod's optional object-parser JIT probes new Function, producing an enforcing
// CSP violation even when its fallback succeeds. All browser entrypoints use
// the interpreter; validation semantics are identical. Vite aliases the exact
// 'zod' import here so split chunks also initialize it before creating schemas.
z.config({ jitless: true });
