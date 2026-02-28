export function normalizeServerTarget(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

export function toWebSocketUrl(serverTarget: string): string {
  const base = normalizeServerTarget(serverTarget);
  if (base.startsWith("wss://") || base.startsWith("ws://")) {
    return `${base}/v1/ws`;
  }
  if (base.startsWith("https://")) {
    return `wss://${base.slice("https://".length)}/v1/ws`;
  }
  if (base.startsWith("http://")) {
    return `ws://${base.slice("http://".length)}/v1/ws`;
  }
  return `ws://${base}/v1/ws`;
}
