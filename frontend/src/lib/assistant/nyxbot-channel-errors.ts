const channelErrors: Record<string, string> = {
  native_gateway_retry_rate_limited: "Wait ten minutes after the last gateway attempt before retrying.",
  native_gateway_retry_busy: "A gateway attempt is already in progress or this channel changed. Refresh its status before retrying.",
  native_gateway_retry_not_available: "Native retry is available only for an active Discord bot currently on NyxID relay.",
  owner_verification_required: "Link your own Discord account to this bot before retrying native activation.",
  answering: "The bot is answering a message. Retry after it finishes.",
  flag_changed: "The native gateway flag is not enabled for the linking owner. Ask an administrator to check it.",
  gateway_not_authenticated:
    "The channel gateway could not authenticate NyxBot. Try reconnecting. If this continues, contact an administrator.",
  gateway_permission_required:
    "The channel gateway has not authorized NyxBot to manage this connection. An administrator needs to check the gateway configuration.",
  gateway_configuration_required:
    "The channel gateway needs administrator configuration. Ask an administrator to check the active cmaeg service and its URL.",
  gateway_unavailable:
    "The channel gateway is temporarily unavailable. Try again shortly.",
  setup_interrupted:
    "This connection changed while it was being set up. Reconnect to try again.",
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

/** Mutation responses may contain a stable gateway code or ordinary NyxID guidance. */
export function nyxBotChannelMutationError(message: string): string {
  message = message.replace(/^Conflict: /, "");
  const gatewayCode = /^(?:Validation error: )?(gateway_[a-z_]+)$/.exec(message)?.[1];
  if (gatewayCode) return nyxBotChannelError(gatewayCode);
  if (Object.hasOwn(channelErrors, message)) {
    return nyxBotChannelError(message);
  }
  return message;
}
