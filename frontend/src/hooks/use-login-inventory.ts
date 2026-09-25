import { api } from "@/lib/api-client";
import { agentKeyOptionsSchema } from "@/schemas/agent-key-login";
import {
  loginCatalogSchema,
  type LoginInventory,
} from "@/lib/login-permissions";
import { userCodeSchema } from "@/schemas/auth-device";
import type { LoginFlow } from "./use-agent-key-login";

export async function fetchLoginInventory(
  flow: LoginFlow,
  code: string,
): Promise<LoginInventory> {
  const [options, catalog] = await Promise.all([
    api.post(`/auth/${flow}/options`, {
      user_code: userCodeSchema.parse(code),
    }),
    api.get("/catalog?include_all=true"),
  ]);
  const parsed = agentKeyOptionsSchema.parse(options);
  return {
    options: parsed,
    connections: parsed.connections ?? [],
    catalog: loginCatalogSchema.parse(catalog).entries,
  };
}
