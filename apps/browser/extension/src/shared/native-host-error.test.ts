import { describe, expect, it } from "vitest";
import { isNativeHostUnavailableError } from "./native-host-error";

describe("isNativeHostUnavailableError", () => {
  it("recognizes Firefox's missing native application error", () => {
    expect(isNativeHostUnavailableError("No such native application com.fetchx.native_host"))
      .toBe(true);
  });

  it("recognizes Chromium's missing native host error", () => {
    expect(isNativeHostUnavailableError("Specified native messaging host not found."))
      .toBe(true);
  });

  it("keeps other disconnects ambiguous", () => {
    expect(isNativeHostUnavailableError("Native host disconnected unexpectedly.")).toBe(false);
    expect(isNativeHostUnavailableError(undefined)).toBe(false);
  });
});
