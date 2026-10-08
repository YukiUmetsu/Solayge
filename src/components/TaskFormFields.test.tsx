import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock, openMock } = vi.hoisted(() => ({
  apiMock: { promptHistory: vi.fn() },
  openMock: vi.fn(),
}));
vi.mock("../api", () => ({ api: apiMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: openMock }));

import {
  TaskFormFields,
  emptyTaskForm,
  type TaskFormState,
} from "./TaskFormFields";
import { makeProject } from "../test/fixtures";

beforeEach(() => {
  apiMock.promptHistory.mockReset().mockResolvedValue([]);
  openMock.mockReset();
});

function Harness({ initial }: { initial?: Partial<TaskFormState> }) {
  const [value, setValue] = useState<TaskFormState>(() => ({
    ...emptyTaskForm("autonomous"),
    ...initial,
  }));
  return (
    <TaskFormFields
      project={makeProject()}
      tasks={[]}
      value={value}
      onChange={(patch) => setValue((v) => ({ ...v, ...patch }))}
    />
  );
}

describe("emptyTaskForm", () => {
  it("starts as an agent task with no attachments", () => {
    const form = emptyTaskForm("readonly");
    expect(form.kind).toBe("agent");
    expect(form.attachments).toEqual([]);
    expect(form.profile).toBe("readonly");
  });
});

describe("TaskFormFields", () => {
  it("attaches files to an agent prompt", async () => {
    const user = userEvent.setup();
    openMock.mockResolvedValue(["/tmp/a.png", "/tmp/b.png"]);
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: /attach files/i }));
    expect(await screen.findByText("a.png")).toBeInTheDocument();
    expect(screen.getByText("b.png")).toBeInTheDocument();
  });

  it("hides attachments for a command task", async () => {
    render(<Harness initial={{ kind: "shell", command: "pnpm test" }} />);

    expect(screen.queryByRole("button", { name: /attach files/i })).toBeNull();
    // The command textarea is shown instead.
    expect(screen.getByDisplayValue("pnpm test")).toBeInTheDocument();
  });

  it("shows the prompt textarea for an agent task", () => {
    render(<Harness />);
    expect(screen.getByPlaceholderText(/full instruction/i)).toBeInTheDocument();
  });
});
