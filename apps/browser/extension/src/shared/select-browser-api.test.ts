import { describe, expect, it } from "vitest";

import { selectBrowserApi } from "./select-browser-api";

describe("browser API selection", () => {
  it("uses Firefox's Promise-based browser namespace when available", () => {
    const firefoxApi = { runtime: { namespace: "browser" } };
    const chromiumApi = { runtime: { namespace: "chrome" } };

    expect(selectBrowserApi(firefoxApi, chromiumApi)).toBe(firefoxApi);
  });

  it("uses the Chromium namespace when Firefox's namespace is absent", () => {
    const chromiumApi = { runtime: { namespace: "chrome" } };

    expect(selectBrowserApi(undefined, chromiumApi)).toBe(chromiumApi);
  });
});
