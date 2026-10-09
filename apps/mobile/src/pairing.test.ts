import { describe, expect, it } from "vitest";

import {
  formatVerificationCode,
  isExpired,
  isManualCodeComplete,
  normalizeManualCode,
  scanErrorMessage,
} from "./pairing";

describe("manual pairing code", () => {
  it("uppercases, strips invalid characters, and groups the code", () => {
    expect(normalizeManualCode(" abcd efgh ")).toBe("ABCD-EFGH");
    expect(normalizeManualCode("abcd-efgh")).toBe("ABCD-EFGH");
    expect(normalizeManualCode("ab!cd@ef$gh")).toBe("ABCD-EFGH");
    // Extra characters are dropped beyond eight.
    expect(normalizeManualCode("abcdefghij")).toBe("ABCD-EFGH");
  });

  it("does not group an incomplete code", () => {
    expect(normalizeManualCode("abc")).toBe("ABC");
    expect(isManualCodeComplete("abc")).toBe(false);
    expect(isManualCodeComplete("abcd")).toBe(false);
  });

  it("accepts a complete code with or without the separator", () => {
    expect(isManualCodeComplete("ABCD-EFGH")).toBe(true);
    expect(isManualCodeComplete("abcdefgh")).toBe(true);
  });
});

describe("verification code formatting", () => {
  it("groups six digits as 3 + 3 and ignores spacing", () => {
    expect(formatVerificationCode("123456")).toBe("123 456");
    expect(formatVerificationCode("123 456")).toBe("123 456");
    expect(formatVerificationCode("12")).toBe("12");
  });
});

describe("expiry", () => {
  it("treats a challenge as expired at or after its timestamp", () => {
    expect(isExpired(1000, 999_999)).toBe(false);
    expect(isExpired(1000, 1_000_000)).toBe(true);
    expect(isExpired(1000, 1_000_001)).toBe(true);
  });

  it("never treats a missing timestamp as expired", () => {
    expect(isExpired(Number.NaN)).toBe(false);
  });
});

describe("scan error messages", () => {
  it("suggests the manual code for every failure mode", () => {
    expect(scanErrorMessage("permission denied")).toMatch(/pairing code/i);
    expect(scanErrorMessage("camera not found")).toMatch(/pairing code/i);
    expect(scanErrorMessage(undefined)).toMatch(/pairing code/i);
  });
});
