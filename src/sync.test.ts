import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { setAudioNormalizationSettings } = vi.hoisted(() => ({
  setAudioNormalizationSettings: vi.fn(),
}));

vi.mock("./api", () => ({
  tauriAPI: { setAudioNormalizationSettings },
}));

import { applySettings } from "./sync";

describe("settings sync migration", () => {
  beforeEach(() => {
    setAudioNormalizationSettings.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it.each([
    ["snake case", { audio_normalization_enabled: true }, true],
    ["camel case", { audioNormalizationEnabled: false }, false],
  ])("applies enabled-only settings from %s payloads", async (_, settings, expected) => {
    await applySettings(settings);

    expect(setAudioNormalizationSettings).toHaveBeenCalledWith(expected);
  });

  it("ignores a legacy strict-only settings payload", async () => {
    await applySettings({ audio_normalization_strict_mode: true });

    expect(setAudioNormalizationSettings).not.toHaveBeenCalled();
  });
});
