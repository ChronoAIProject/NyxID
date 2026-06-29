// Service-icon registry: maps each catalog `slug` to the icon component that
// renders on its tile in the "Add Service" dialog's `CatalogGrid`. The
// `FallbackIcon` (Lucide Globe with a `data-fallback="true"` test hook) renders
// for unknown slugs.
//
// 2-tone discipline (checked in the test `add-key-dialog.test.tsx`):
//  - The 29 known-slugs in `SPEC_CATALOG_SLUGS` carry `data-slug="<slug>"` on
//    the rendered `<svg>` (or, for composites, on the brand `<svg>` nested
//    inside the wrapper `<span>`).
//  - `FallbackIcon` carries `data-fallback="true"` instead — it MUST NOT carry
//    any `data-slug` attribute.
//  - The accent class `text-nyx-secondary-400` lives only on the Lucide badge
//    wrapper of composite icons; the primary brand glyph stays
//    `currentColor` only.

import type { ComponentType, SVGProps } from "react";
import { Globe } from "lucide-react";

import LlmOpenaiIcon from "./llm-openai";
import LlmOpenaiCodexIcon from "./llm-openai-codex";
import LlmAnthropicIcon from "./llm-anthropic";
import LlmGoogleAiIcon from "./llm-google-ai";
import LlmMistralIcon from "./llm-mistral";
import LlmCohereIcon from "./llm-cohere";
import LlmDeepSeekIcon from "./llm-deepseek";
import LlmOpenClawIcon from "./llm-openclaw";

import ApiTwitterIcon from "./api-twitter";
import ApiGoogleIcon from "./api-google";
import ApiGoogleCloudIcon from "./api-google-cloud";
import ApiGithubIcon from "./api-github";
import ApiGithubPatIcon from "./api-github-pat";
import ApiFacebookIcon from "./api-facebook";
import ApiDiscordIcon from "./api-discord";
import ApiDiscordBotIcon from "./api-discord-bot";
import ApiSpotifyIcon from "./api-spotify";
import ApiSlackIcon from "./api-slack";
import ApiSlackBotIcon from "./api-slack-bot";
import ApiMicrosoftIcon from "./api-microsoft";
import ApiTiktokIcon from "./api-tiktok";
import ApiTwitchIcon from "./api-twitch";
import ApiRedditIcon from "./api-reddit";
import ApiLarkIcon from "./api-lark";
import ApiLarkBotIcon from "./api-lark-bot";
import ApiFeishuIcon from "./api-feishu";
import ApiFeishuBotIcon from "./api-feishu-bot";
import ApiTelegramBotIcon from "./api-telegram-bot";

import AwsCostExplorerIcon from "./aws-cost-explorer";

export type ServiceIconProps = { className?: string };

export type IconComponent = ComponentType<ServiceIconProps>;

// The 29 slugs seeded in `backend/src/services/provider_service.rs`
// (lines ~1818-2218) — authoritative the test setup asserts against.
export const SPEC_CATALOG_SLUGS = [
  "llm-openai",
  "llm-openai-codex",
  "llm-anthropic",
  "llm-google-ai",
  "llm-mistral",
  "llm-cohere",
  "llm-deepseek",
  "llm-openclaw",

  "api-twitter",
  "api-google",
  "api-google-cloud",
  "api-github",
  "api-github-pat",
  "api-facebook",
  "api-discord",
  "api-discord-bot",
  "api-spotify",
  "api-slack",
  "api-slack-bot",
  "api-microsoft",
  "api-tiktok",
  "api-twitch",
  "api-reddit",
  "api-lark",
  "api-lark-bot",
  "api-feishu",
  "api-feishu-bot",
  "api-telegram-bot",

  "aws-cost-explorer",
] as const;

type Slug = (typeof SPEC_CATALOG_SLUGS)[number];

export const SERVICE_ICONS: Readonly<Record<string, IconComponent>> = {
  "llm-openai": LlmOpenaiIcon,
  "llm-openai-codex": LlmOpenaiCodexIcon,
  "llm-anthropic": LlmAnthropicIcon,
  "llm-google-ai": LlmGoogleAiIcon,
  "llm-mistral": LlmMistralIcon,
  "llm-cohere": LlmCohereIcon,
  "llm-deepseek": LlmDeepSeekIcon,
  "llm-openclaw": LlmOpenClawIcon,

  "api-twitter": ApiTwitterIcon,
  "api-google": ApiGoogleIcon,
  "api-google-cloud": ApiGoogleCloudIcon,
  "api-github": ApiGithubIcon,
  "api-github-pat": ApiGithubPatIcon,
  "api-facebook": ApiFacebookIcon,
  "api-discord": ApiDiscordIcon,
  "api-discord-bot": ApiDiscordBotIcon,
  "api-spotify": ApiSpotifyIcon,
  "api-slack": ApiSlackIcon,
  "api-slack-bot": ApiSlackBotIcon,
  "api-microsoft": ApiMicrosoftIcon,
  "api-tiktok": ApiTiktokIcon,
  "api-twitch": ApiTwitchIcon,
  "api-reddit": ApiRedditIcon,
  "api-lark": ApiLarkIcon,
  "api-lark-bot": ApiLarkBotIcon,
  "api-feishu": ApiFeishuIcon,
  "api-feishu-bot": ApiFeishuBotIcon,
  "api-telegram-bot": ApiTelegramBotIcon,

  "aws-cost-explorer": AwsCostExplorerIcon,
} satisfies Readonly<Record<Slug, IconComponent>>;

// `data-fallback="true"` lets the test hook recognize fallbacks when an
// unknown slug is rendered.
export function FallbackIcon({ className }: ServiceIconProps) {
  return (
    <Globe
      className={className}
      aria-hidden="true"
      {...({ "data-fallback": "true" } as SVGProps<SVGSVGElement>)}
    />
  );
}

export function ServiceIcon({
  slug,
  className,
}: {
  slug: string;
  className?: string;
}) {
  const Icon = SERVICE_ICONS[slug] ?? FallbackIcon;
  return <Icon className={className} />;
}
