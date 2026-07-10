import { describe, expect, it } from "vitest";

import { errorMessage } from "./use-error-toast";

describe("errorMessage", () => {
  it("uses an Error message when available", () => {
    expect(errorMessage(new Error("Request failed"), "Fallback")).toBe("Request failed");
  });

  it("uses the localized fallback for unknown errors", () => {
    expect(errorMessage(null, "请求失败")).toBe("请求失败");
  });
});
