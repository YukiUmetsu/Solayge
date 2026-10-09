import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Project, Worktree, WorktreeDetail } from "../types";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    worktreeDetail: vi.fn(),
    worktreeCommitFiles: vi.fn(),
    gitRevertCommit: vi.fn(),
    gitMergeBranch: vi.fn(),
    gitRemoveWorktree: vi.fn(),
    gitDeleteBranch: vi.fn(),
    gitDeleteRemoteBranch: vi.fn(),
  },
}));
vi.mock("../api", () => ({ api: apiMock }));

import { GitWorktrees } from "./GitPanels";

const PROJECT: Project = {
  path: "/repo/app",
  name: "app",
  added_at: 0,
  env_vars: [],
  skills: [],
};

const WORKTREE: Worktree = {
  path: "/repo/app/.dev-tools/worktrees/wt1",
  branch: "devtools/wt1",
  head: "abcdef1234567890",
  is_main: false,
};

const DETAIL: WorktreeDetail = {
  path: WORKTREE.path,
  branch: "devtools/wt1",
  base: "main",
  head: WORKTREE.head,
  commit_count: 1,
  files_changed: 1,
  additions: 3,
  deletions: 1,
  last_commit_at: 1_700_000_000,
  last_commit_subject: "Add the thing",
  dirty: false,
  commits: [
    {
      id: "abcdef1234567890",
      short_id: "abcdef1",
      subject: "Add the thing",
      author: "Dev",
      committed_at: 1_700_000_000,
      files_changed: 1,
      additions: 3,
      deletions: 1,
    },
  ],
};

beforeEach(() => {
  apiMock.worktreeDetail.mockReset().mockResolvedValue(DETAIL);
  apiMock.worktreeCommitFiles.mockReset().mockResolvedValue([
    { path: "src/thing.ts", status: "modified", additions: 3, deletions: 1 },
  ]);
  apiMock.gitRevertCommit.mockReset().mockResolvedValue("Reverted commit abcdef1.");
});

function renderWorktrees(onChanged = () => {}) {
  return render(
    <GitWorktrees
      project={PROJECT}
      branches={[]}
      defaultBranch="main"
      worktrees={[WORKTREE]}
      loading={false}
      error={null}
      onReload={() => {}}
      onChanged={onChanged}
    />,
  );
}

describe("Git worktree details", () => {
  it("keeps details hidden until expanded, then shows commits and files", async () => {
    const user = userEvent.setup();
    renderWorktrees();

    // The default view is compact: no detail fetch until the user asks.
    expect(apiMock.worktreeDetail).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Details/ }));
    await waitFor(() => expect(apiMock.worktreeDetail).toHaveBeenCalledTimes(1));

    expect(await screen.findByText("Add the thing")).toBeInTheDocument();
    expect(screen.getByText(/1 commit/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Add the thing/ }));
    expect(await screen.findByText("src/thing.ts")).toBeInTheDocument();
    expect(screen.getByText("modified")).toBeInTheDocument();
  });

  it("undoes a commit by reverting it after confirmation", async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderWorktrees(onChanged);

    await user.click(screen.getByRole("button", { name: /Details/ }));
    await user.click(await screen.findByRole("button", { name: /Add the thing/ }));
    await user.click(screen.getByRole("button", { name: /Undo/ }));

    await waitFor(() =>
      expect(apiMock.gitRevertCommit).toHaveBeenCalledWith(WORKTREE.path, "abcdef1234567890"),
    );
    expect(onChanged).toHaveBeenCalled();
  });

  it("does not revert when the confirmation is declined", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    renderWorktrees();

    await user.click(screen.getByRole("button", { name: /Details/ }));
    await user.click(await screen.findByRole("button", { name: /Add the thing/ }));
    await user.click(screen.getByRole("button", { name: /Undo/ }));

    expect(apiMock.gitRevertCommit).not.toHaveBeenCalled();
  });

  it("keeps the commit list visible when a revert fails", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    apiMock.gitRevertCommit.mockRejectedValueOnce(
      new Error("the working tree has uncommitted changes"),
    );
    renderWorktrees();

    await user.click(screen.getByRole("button", { name: /Details/ }));
    await user.click(await screen.findByRole("button", { name: /Add the thing/ }));
    await user.click(screen.getByRole("button", { name: /Undo/ }));

    expect(await screen.findByText(/uncommitted changes/)).toBeInTheDocument();
    // The failure must not replace the detail view, so the user can retry.
    expect(screen.getByRole("button", { name: /Add the thing/ })).toBeInTheDocument();
  });
});
