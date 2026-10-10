import { describe, expect, it } from "vitest";
import {
  DEFAULT_NOTIFICATION_SETTINGS,
  DEFAULT_SOUND_FOR,
  soundValue,
} from "./notifications";
import type { NotifyKind } from "../types";

describe("notification sound mapping", () => {
  it("maps each kind to its configured sound", () => {
    const n = {
      ...DEFAULT_NOTIFICATION_SETTINGS,
      complete_sound: "complete",
      failed_sound: "failed",
      review_sound: "review",
      attention_sound: "attention",
    };
    expect(soundValue("task_complete", n)).toBe("complete");
    expect(soundValue("system", n)).toBe("complete");
    expect(soundValue("task_failed", n)).toBe("failed");
    expect(soundValue("task_review", n)).toBe("review");
    expect(soundValue("needs_attention", n)).toBe("attention");
  });

  it("returns null when a sound is disabled", () => {
    const n = { ...DEFAULT_NOTIFICATION_SETTINGS, failed_sound: null };
    expect(soundValue("task_failed", n)).toBeNull();
  });

  it("has a default preset for every notify kind", () => {
    const kinds: NotifyKind[] = [
      "task_complete",
      "task_failed",
      "task_review",
      "needs_attention",
      "system",
    ];
    for (const k of kinds) {
      expect(DEFAULT_SOUND_FOR[k]).toBeTruthy();
    }
  });
});
