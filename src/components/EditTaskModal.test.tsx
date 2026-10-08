import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    updateTask: vi.fn(),
    promptHistory: vi.fn().mockResolvedValue([]),
  },
}));
vi.mock("../api", () => ({ api: apiMock }));

import { EditTaskModal } from "./EditTaskModal";
import { makeProject, makeSnapshot, makeTask } from "../test/fixtures";

beforeEach(() => {
  apiMock.updateTask.mockReset().mockResolvedValue(makeSnapshot());
  apiMock.promptHistory.mockReset().mockResolvedValue([]);
});

describe("Edit task — attachments", () => {
  it("shows the task's current attachments", () => {
    render(
      <EditTaskModal
        task={makeTask({ attachments: ["/tmp/old.txt"] })}
        project={makeProject()}
        tasks={[]}
        onClose={vi.fn()}
        onSaved={vi.fn()}
      />,
    );
    expect(screen.getByText("old.txt")).toBeInTheDocument();
    expect(screen.getByText("/tmp/old.txt")).toBeInTheDocument();
  });

  it("saves an empty attachment list when they are removed", async () => {
    const user = userEvent.setup();
    render(
      <EditTaskModal
        task={makeTask({ attachments: ["/tmp/old.txt"] })}
        project={makeProject()}
        tasks={[]}
        onClose={vi.fn()}
        onSaved={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: /remove old\.txt/i }));
    await user.click(screen.getByRole("button", { name: /save changes/i }));

    await waitFor(() => expect(apiMock.updateTask).toHaveBeenCalled());
    const [, patch] = apiMock.updateTask.mock.calls[0];
    expect(patch.attachments).toEqual([]);
  });
});
