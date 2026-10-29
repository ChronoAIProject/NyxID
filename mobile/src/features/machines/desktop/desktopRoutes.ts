export type MachineDesktopParams = {
  nodeId: string;
  conversationId?: string;
  contextId?: string;
  display?: 'secure' | 'dev';
};

export function desktopQuery(params: MachineDesktopParams): string {
  const query = new URLSearchParams();
  if (params.conversationId) query.set('conversation_id', params.conversationId);
  if (params.contextId) query.set('context_id', params.contextId);
  if (params.display === 'dev') query.set('display', 'dev');
  const value = query.toString();
  return value ? `?${value}` : '';
}

export function desktopSocketUrl(apiBase: string, params: MachineDesktopParams): string {
  const url = new URL(apiBase.replace(/\/+$/, '').replace(/\/mobile$/, ''));
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error('Desktop API URL unavailable');
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = `${url.pathname.replace(/\/+$/, '')}/assistant/nyxagent/machines/${encodeURIComponent(params.nodeId)}/desktop`;
  url.search = desktopQuery(params); url.hash = '';
  return url.toString();
}
