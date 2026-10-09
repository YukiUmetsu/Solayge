import { describe, expect, it, vi } from "vitest";
import { screen } from "@testing-library/react";

import { MockRemoteClient } from "../client/mock";
import { RemoteError } from "../client/remote";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { ProjectDetail } from "./ProjectDetail";
import { Projects } from "./Projects";

describe("Projects", () => {
  it("lists projects with running and pending counts", async () => {
    renderWithClient(
      <Projects onOpenProject={() => {}} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
    );
    expect(await screen.findByText("Solayge")).toBeInTheDocument();
    expect(screen.getByText("FileQuell")).toBeInTheDocument();
    expect(screen.getAllByText(/running/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/need input/).length).toBeGreaterThan(0);
  });

  it("shows an empty state with no projects", async () => {
    renderWithClient(
      <Projects onOpenProject={() => {}} />,
      new MockRemoteClient({ empty: true, now: () => FIXED_NOW }),
    );
    expect(await screen.findByText("No projects yet")).toBeInTheDocument();
  });

  it("shows project detail with tasks and changed files", async () => {
    renderWithClient(
      <ProjectDetail projectId="/repo/solayge" onOpenSession={() => {}} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
    );

    expect(await screen.findByRole("heading", { name: "Solayge" })).toBeInTheDocument();
    expect(screen.getByText("Refactor UI")).toBeInTheDocument();
    expect(screen.getByText("Fix navigation")).toBeInTheDocument();
    // Changed files are aggregated for the project's sessions.
    expect(await screen.findByText("src/SessionView.tsx")).toBeInTheDocument();
    expect(screen.getByText("+42 / −9")).toBeInTheDocument();
  });

  it("surfaces a failed changed-files read instead of claiming there are no changes", async () => {
    const c = new MockRemoteClient({ now: () => FIXED_NOW });
    vi.spyOn(c, "filesChangedSummary").mockRejectedValue(
      new RemoteError("transport", "offline"),
    );
    renderWithClient(<ProjectDetail projectId="/repo/solayge" onOpenSession={() => {}} />, c);

    expect(await screen.findByText("Couldn't load changed files.")).toBeInTheDocument();
    expect(screen.queryByText("No changes in this project.")).toBeNull();
  });
});
