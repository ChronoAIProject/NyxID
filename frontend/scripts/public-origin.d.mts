export const DEFAULT_PUBLIC_ORIGIN: string;
export function getPublicOrigin(env?: NodeJS.ProcessEnv): string;
export function publicUrl(pathname: string, env?: NodeJS.ProcessEnv): string;
export function buildSitemap(urls: readonly string[], env?: NodeJS.ProcessEnv): string;
export function buildRobots(source: string, env?: NodeJS.ProcessEnv): string;
export function buildLegalMarkdown(source: string, env?: NodeJS.ProcessEnv): string;

