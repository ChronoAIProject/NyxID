const channelErrors: Record<string, string> = {
  gateway_not_authenticated:
    "The channel gateway could not authenticate NyxBot. Try reconnecting. If this continues, contact an administrator.",
  gateway_permission_required:
    "The channel gateway has not authorized NyxBot to manage this connection. An administrator needs to check the gateway configuration.",
  gateway_unavailable:
    "The channel gateway is temporarily unavailable. Try again shortly.",
  gateway_provider_not_registered:
    "The Agent Event Gateway has not registered NyxBot as a provider yet (operator step). Telegram bots cannot use NyxBot until it is.",
  gateway_rejected_request:
    "The channel gateway rejected this connection request. Contact an administrator.",
  gateway_conflict:
    "The channel gateway reported a conflicting connection. Contact an administrator.",
  gateway_provider_rejected:
    "The channel gateway could not complete setup with NyxBot. Contact an administrator.",
  gateway_invalid_response:
    "The channel gateway returned an invalid connection response. Contact an administrator.",
  issuer_unsupported:
    "NyxBot cannot connect with this server's issuer configuration. Contact an administrator.",
  creator_unavailable:
    "NyxBot could not prepare its channel gateway credential. Contact an administrator.",
  storage_unavailable:
    "NyxBot could not save this connection. Try again shortly.",
  route_key_create_failed:
    "NyxBot could not prepare this bot's connection credential. Try again shortly.",
  route_key_update_failed:
    "NyxBot could not configure this bot's connection credential. Try again shortly.",
  route_create_failed:
    "NyxBot could not create this bot's message route. Try again shortly.",
  org_access_lost:
    "You no longer administer the organization that owns this bot, so NyxBot stopped answering through it.",
};

export function nyxBotChannelError(code: string): string {
  return Object.hasOwn(channelErrors, code)
    ? channelErrors[code]!
    : "This connection failed.";
}
