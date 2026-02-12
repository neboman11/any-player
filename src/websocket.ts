type WsMessage<T = unknown> = {
  event: string;
  data: T;
};

type WsHandler<T> = (data: T) => void;

type ListenerMap = Map<string, Set<WsHandler<unknown>>>;

class BackendSocket {
  private ws: WebSocket | null = null;
  private listeners: ListenerMap = new Map();
  private reconnectTimer: number | null = null;
  private connecting = false;
  // Hard-coded to match the backend websocket server port.
  // If the backend port changes or is configurable, this must be updated.
  private readonly url = "ws://127.0.0.1:8990";

  private connect() {
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.connecting)) {
      return;
    }

    this.connecting = true;
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      this.connecting = false;
    };

    this.ws.onclose = () => {
      this.connecting = false;
      this.ws = null;
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      this.connecting = false;
      if (this.ws) {
        this.ws.close();
      }
    };

    this.ws.onmessage = (event) => {
      if (typeof event.data !== "string") {
        return;
      }

      let parsed: WsMessage | null = null;
      try {
        parsed = JSON.parse(event.data) as WsMessage;
      } catch (err) {
        console.warn("Failed to parse websocket message", err);
        return;
      }

      if (!parsed || typeof parsed.event !== "string") {
        return;
      }

      const handlers = this.listeners.get(parsed.event);
      if (!handlers || handlers.size === 0) {
        return;
      }

      handlers.forEach((handler) => {
        handler(parsed?.data);
      });
    };
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null) {
      return;
    }

    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      if (this.listeners.size > 0) {
        this.connect();
      }
    }, 1000);
  }

  on<T>(event: string, handler: WsHandler<T>) {
    const existing = this.listeners.get(event);
    if (existing) {
      existing.add(handler as WsHandler<unknown>);
    } else {
      this.listeners.set(event, new Set([handler as WsHandler<unknown>]));
    }

    this.connect();

    return () => {
      const handlers = this.listeners.get(event);
      if (!handlers) {
        return;
      }
      handlers.delete(handler as WsHandler<unknown>);
      if (handlers.size === 0) {
        this.listeners.delete(event);
      }
      // Close the websocket if no listeners remain
      if (this.listeners.size === 0 && this.ws) {
        this.ws.close();
        this.ws = null;
        this.connecting = false;
      }
    };
  }
}

export const backendSocket = new BackendSocket();
