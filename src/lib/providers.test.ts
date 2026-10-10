import { describe, expect, it } from "vitest";
import {
  conflictModeLabel,
  editorLabel,
  effectiveConfig,
  providerLabel,
  reviewModeLabel,
  separationConfig,
  separationFromIsolation,
  taskKindLabel,
  taskSeparation,
  taskSeparationDisplay,
} from "./providers";

describe("labels", () => {
  it("names providers", () => {
    expect(providerLabel("opencode")).toBe("opencode");
    expect(providerLabel("claude")).toBe("Claude Code");
    expect(providerLabel(null)).toBe("—");
  });

  it("names editors", () => {
    expect(editorLabel("vscode")).toBe("VS Code");
    expect(editorLabel(undefined)).toBe("System default");
  });

  it("names review and conflict modes", () => {
    expect(reviewModeLabel("autofix")).toBe("Review + auto-fix");
    expect(reviewModeLabel(null)).toBe("Off");
    expect(conflictModeLabel("agent_auto")).toContain("continues");
  });

  it("labels non-agent task kinds", () => {
    expect(taskKindLabel({ kind: "agent" })).toBeNull();
    expect(taskKindLabel({ kind: "shell" })).toBe("shell");
    expect(taskKindLabel({ kind: "git", git_op: "push" })).toBe("push");
    expect(taskKindLabel({ kind: "merge" })).toBe("combine");
  });
});

describe("separation mapping", () => {
  it("maps each choice to isolation + branch mode", () => {
    expect(separationConfig("worktree")).toEqual({
      isolation: "worktree",
      branch_mode: "current",
    });
    expect(separationConfig("branch")).toEqual({
      isolation: "shared",
      branch_mode: "new",
    });
    expect(separationConfig("current")).toEqual({
      isolation: "shared",
      branch_mode: "current",
    });
  });

  it("maps a plan's isolation onto a choice", () => {
    expect(separationFromIsolation("worktree", "current")).toBe("worktree");
    expect(separationFromIsolation("shared", "worktree")).toBe("current");
    expect(separationFromIsolation(null, "branch")).toBe("branch");
  });

  it("derives a task's separation and display", () => {
    expect(taskSeparation({ isolation: "worktree" })).toBe("worktree");
    expect(taskSeparation({ isolation: "shared", branch_mode: "new" })).toBe("branch");
    expect(taskSeparation({ isolation: "shared", branch_mode: "current" })).toBe("current");

    expect(taskSeparationDisplay({ isolation: "worktree", branch: "feat/a" })).toEqual({
      label: "feat/a",
      icon: "branch",
    });
    expect(taskSeparationDisplay({ isolation: "shared", branch_mode: "new" })).toEqual({
      label: "new branch",
      icon: "branch",
    });
    expect(taskSeparationDisplay({ isolation: "shared", branch_mode: "current" })).toEqual({
      label: "current branch",
      icon: "folder",
    });
  });
});

describe("effectiveConfig", () => {
  it("prefers project values, then account defaults", () => {
    const eff = effectiveConfig(
      { provider: null, model: "project-model" },
      { provider: "claude", model: "account-model", review_mode: "report" },
    );
    expect(eff.provider).toBe("claude");
    expect(eff.model).toBe("project-model");
    expect(eff.review_mode).toBe("report");
  });

  it("defaults the provider to opencode and review to off", () => {
    const eff = effectiveConfig({}, {});
    expect(eff.provider).toBe("opencode");
    expect(eff.review_mode).toBe("off");
  });

  it("defaults the reviewer to the task's provider and model", () => {
    const eff = effectiveConfig(
      { provider: "codex", model: "gpt-5-codex" },
      {},
    );
    expect(eff.review_provider).toBe("codex");
    expect(eff.review_model).toBe("gpt-5-codex");
  });
});
