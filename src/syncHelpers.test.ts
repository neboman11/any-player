import { describe, expect, it } from "vitest";
import { normalizeServerTarget, toWebSocketUrl } from "./syncHelpers";

describe("syncHelpers", () => {
  it("normalizes server target by trimming whitespace and trailing slashes", () => {
    expect(normalizeServerTarget("  https://sync.example.com///  ")).toBe(
      "https://sync.example.com",
    );
  });

  it("maps https targets to secure websocket URLs", () => {
    expect(toWebSocketUrl("https://sync.example.com/")).toBe(
      "wss://sync.example.com/v1/ws",
    );
  });

  it("maps http targets to websocket URLs", () => {
    expect(toWebSocketUrl("http://localhost:8787")).toBe(
      "ws://localhost:8787/v1/ws",
    );
  });

  it("defaults host-only targets to websocket URLs", () => {
    expect(toWebSocketUrl("sync.local:9000")).toBe(
      "ws://sync.local:9000/v1/ws",
    );
  });

  it("passes through wss targets unchanged", () => {
    expect(toWebSocketUrl("wss://sync.example.com")).toBe(
      "wss://sync.example.com/v1/ws",
    );
  });

  it("passes through ws targets unchanged", () => {
    expect(toWebSocketUrl("ws://localhost:9000")).toBe(
      "ws://localhost:9000/v1/ws",
    );
  });

  it("appends encoded token as websocket query parameter", () => {
    expect(toWebSocketUrl("https://sync.example.com", "token with space")).toBe(
      "wss://sync.example.com/v1/ws?token=token+with+space",
    );
  });

  it("builds websocket URL from serverTarget with existing path and query", () => {
    expect(toWebSocketUrl("https://host.example.com/path?x=y")).toBe(
      "wss://host.example.com/v1/ws",
    );
  });
});
