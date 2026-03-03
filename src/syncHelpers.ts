export function normalizeServerTarget(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

export function toWebSocketUrl(
  serverTarget: string,
  authToken?: string,
): string {
  const base = normalizeServerTarget(serverTarget);
  const tokenQuery = authToken?.trim()
    ? `?token=${encodeURIComponent(authToken.trim())}`
    : "";
  if (base.startsWith("wss://") || base.startsWith("ws://")) {
    return `${base}/v1/ws${tokenQuery}`;
  }
  if (base.startsWith("https://")) {
    return `wss://${base.slice("https://".length)}/v1/ws${tokenQuery}`;
  }
  if (base.startsWith("http://")) {
    return `ws://${base.slice("http://".length)}/v1/ws${tokenQuery}`;
  }
  return `ws://${base}/v1/ws${tokenQuery}`;
}
