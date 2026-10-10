import { describe, expect, it } from "vitest";

import { TABS } from "./tabs";

describe("bottom tab model", () => {
  it("has the three primary destinations in order", () => {
    expect(TABS.map((tab) => tab.id)).toEqual(["sessions", "inbox", "projects"]);
    expect(TABS.map((tab) => tab.label)).toEqual(["Sessions", "Inbox", "Projects"]);
  });

  it("gives every tab a heading, empty state, and icon", () => {
    for (const tab of TABS) {
      expect(tab.title.length).toBeGreaterThan(0);
      expect(tab.empty.length).toBeGreaterThan(0);
      expect(tab.icon.length).toBeGreaterThan(0);
    }
  });

  it("uses unique ids", () => {
    expect(new Set(TABS.map((tab) => tab.id)).size).toBe(TABS.length);
  });
});
