import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock, openMock } = vi.hoisted(() => ({
  apiMock: {
    createTasks: vi.fn(),
    promptHistory: vi.fn(),
  },
  openMock: vi.fn(),
}));
vi.mock("../api", () => ({ api: apiMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

import { NewTaskModal } from "./NewTaskModal";
import { makeProject, makeSnapshot } from "../test/fixtures";

beforeEach(() => {
  apiMock.createTasks.mockReset().mockResolvedValue(makeSnapshot());
  apiMock.promptHistory.mockReset().mockResolvedValue([]);
  openMock.mockReset();
});

describe("New task — attachments", () => {
  it("attaches multiple files and sends them with the task", async () => {
    const user = userEvent.setup();
    openMock.mockResolvedValue(["/tmp/diagram.png", "/tmp/spec.md"]);
    const onCreated = vi.fn();
    render(
      <NewTaskModal
        project={makeProject()}
        tasks={[]}
        onClose={vi.fn()}
        onCreated={onCreated}
      />,
    );

    await user.type(
      screen.getByPlaceholderText(/full instruction/i),
      "Use the attached files.",
    );
    await user.click(screen.getByRole("button", { name: /attach files/i }));

    expect(await screen.findByText("diagram.png")).toBeInTheDocument();
    expect(screen.getByText("spec.md")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /create task/i }));

    await waitFor(() => expect(apiMock.createTasks).toHaveBeenCalled());
    const [, tasks] = apiMock.createTasks.mock.calls[0];
    expect(tasks).toHaveLength(1);
    expect(tasks[0]).toMatchObject({
      prompt: "Use the attached files.",
      kind: "agent",
      attachments: ["/tmp/diagram.png", "/tmp/spec.md"],
    });
    expect(onCreated).toHaveBeenCalled();
  });

  it("removes an attachment before creating", async () => {
    const user = userEvent.setup();
    openMock.mockResolvedValue(["/tmp/a.txt", "/tmp/b.txt"]);
    render(
      <NewTaskModal
        project={makeProject()}
        tasks={[]}
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    await user.type(screen.getByPlaceholderText(/full instruction/i), "Hi");
    await user.click(screen.getByRole("button", { name: /attach files/i }));
    await screen.findByText("a.txt");
    await user.click(
      screen.getByRole("button", { name: /remove a\.txt/i }),
    );
    expect(screen.queryByText("a.txt")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /create task/i }));
    await waitFor(() => expect(apiMock.createTasks).toHaveBeenCalled());
    const [, tasks] = apiMock.createTasks.mock.calls[0];
    expect(tasks[0].attachments).toEqual(["/tmp/b.txt"]);
  });
});
