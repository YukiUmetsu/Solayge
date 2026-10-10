import { beforeEach, describe, expect, it, vi } from "vitest";

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

import {
  fileName,
  mergeAttachments,
  pickFiles,
  removeAttachment,
} from "./attachments";

describe("fileName", () => {
  it("returns the last path segment", () => {
    expect(fileName("/tmp/a/b/report.pdf")).toBe("report.pdf");
    expect(fileName("C:\\Users\\me\\notes.txt")).toBe("notes.txt");
    expect(fileName("bare.txt")).toBe("bare.txt");
  });

  it("falls back to the whole path when there is no name", () => {
    expect(fileName("/tmp/")).toBe("/tmp/");
  });
});

describe("mergeAttachments", () => {
  it("appends new files", () => {
    expect(mergeAttachments(["/a.txt"], ["/b.txt"])).toEqual([
      "/a.txt",
      "/b.txt",
    ]);
  });

  it("trims, drops blanks, and de-duplicates", () => {
    expect(
      mergeAttachments(["/a.txt"], [" /a.txt ", "  ", "/b.txt", "/b.txt"]),
    ).toEqual(["/a.txt", "/b.txt"]);
  });

  it("does not mutate its inputs", () => {
    const existing = ["/a.txt"];
    mergeAttachments(existing, ["/b.txt"]);
    expect(existing).toEqual(["/a.txt"]);
  });
});

describe("removeAttachment", () => {
  it("removes exactly the given path", () => {
    expect(removeAttachment(["/a.txt", "/b.txt"], "/a.txt")).toEqual(["/b.txt"]);
  });
});

describe("pickFiles", () => {
  beforeEach(() => openMock.mockReset());

  it("asks for multiple files and returns the paths", async () => {
    openMock.mockResolvedValue(["/a.png", "/b.txt"]);
    await expect(pickFiles()).resolves.toEqual(["/a.png", "/b.txt"]);
    expect(openMock).toHaveBeenCalledWith(
      expect.objectContaining({ multiple: true, directory: false }),
    );
  });

  it("accepts a single string result", async () => {
    openMock.mockResolvedValue("/only.txt");
    await expect(pickFiles()).resolves.toEqual(["/only.txt"]);
  });

  it("returns an empty list when cancelled", async () => {
    openMock.mockResolvedValue(null);
    await expect(pickFiles()).resolves.toEqual([]);
  });
});
