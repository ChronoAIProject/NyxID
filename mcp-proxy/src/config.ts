import { z } from 'zod';

const configSchema = z.object({
  nyxidUrl: z.string().url(),
  nyxidClientId: z.string().min(1),
  nyxidClientSecret: z.string().min(1),
  mcpPort: z.number().int().positive(),
});

export type Config = z.infer<typeof configSchema>;

export function loadConfig(): Config {
  const result = configSchema.safeParse({
    nyxidUrl: process.env.NYXID_URL,
    nyxidClientId: process.env.NYXID_CLIENT_ID,
    nyxidClientSecret: process.env.NYXID_CLIENT_SECRET,
    mcpPort: process.env.MCP_PORT
      ? parseInt(process.env.MCP_PORT, 10)
      : 3001,
  });

  if (!result.success) {
    const errors = result.error.issues
      .map((i) => `  ${i.path.join('.')}: ${i.message}`)
      .join('\n');
    throw new Error(`Invalid configuration:\n${errors}`);
  }

  return result.data;
}
