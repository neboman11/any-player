export function normalizeServerTarget(value: string): string {
  return value.trim().replace(/\/+$/, "");
}

export function toWebSocketUrl(
  serverTarget: string,
  authToken?: string,
): string {
  const base = normalizeServerTarget(serverTarget);

  let httpBase: string;
  let isSecure: boolean;

  if (base.startsWith("wss://")) {
    httpBase = `https://${base.slice("wss://".length)}`;
    isSecure = true;
  } else if (base.startsWith("ws://")) {
    httpBase = `http://${base.slice("ws://".length)}`;
    isSecure = false;
  } else if (base.startsWith("https://")) {
    httpBase = base;
    isSecure = true;
  } else if (base.startsWith("http://")) {
    httpBase = base;
    isSecure = false;
  } else {
    httpBase = `http://${base}`;
    isSecure = false;
  }

  const url = new URL("/v1/ws", httpBase);
  url.protocol = isSecure ? "wss:" : "ws:";

  const token = authToken?.trim();
  if (token) {
    url.searchParams.set("token", token);
  }

  return url.toString();
}
