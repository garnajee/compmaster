import { afterEach, describe, expect, it, vi } from "vitest";
import { api, setToken } from "./api";
afterEach(() => vi.unstubAllGlobals());
describe("API safety", () => {
  it("sends CSRF for writes and propagates conflict diagnostics", async () => {
    setToken("csrf");
    const fetch = vi
      .fn()
      .mockResolvedValue({
        ok: false,
        status: 409,
        json: async () => ({ error: "Le fichier a changé" }),
      });
    vi.stubGlobal("fetch", fetch);
    await expect(
      api("/projects/p/file", "PUT", {
        revision: "old",
        content: "services: {}",
      }),
    ).rejects.toThrow("Le fichier a changé");
    expect(fetch.mock.calls[0][1].headers["X-CSRF-Token"]).toBe("csrf");
    expect(JSON.parse(fetch.mock.calls[0][1].body).revision).toBe("old");
  });
});
