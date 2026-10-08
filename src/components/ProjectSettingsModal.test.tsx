import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    projectRemote: vi.fn(),
    projectSecrets: vi.fn(),
    updateProjectConfig: vi.fn(),
    openInEditor: vi.fn(),
    openExternal: vi.fn(),
    createTasks: vi.fn(),
  },
}));
vi.mock("../api", () => ({ api: apiMock }));

import { ProjectSettingsModal } from "./ProjectSettingsModal";
import type { Project } from "../types";
import {
  makeProject,
  makeSettings,
  makeSnapshot,
  makeTools,
} from "../test/fixtures";

function renderModal(systemPrompt: Project["system_prompt"]) {
  const onSaved = vi.fn();
  const onClose = vi.fn();
  render(
    <ProjectSettingsModal
      project={makeProject({ system_prompt: systemPrompt ?? null })}
      settings={makeSettings()}
      tools={makeTools()}
      onClose={onClose}
      onSaved={onSaved}
    />,
  );
  return { onSaved, onClose };
}

beforeEach(() => {
  apiMock.projectRemote.mockReset().mockResolvedValue(null);
  apiMock.projectSecrets.mockReset().mockResolvedValue([]);
  apiMock.updateProjectConfig.mockReset().mockResolvedValue(makeSnapshot());
});

describe("Project settings — Prompt tab", () => {
  it("lets you type and change the position without enabling first", async () => {
    const user = userEvent.setup();
    renderModal(null);

    await user.click(screen.getByRole("button", { name: /prompt/i }));

    const textarea = await screen.findByPlaceholderText(/this project uses pnpm/i);
    const select = screen.getByRole("combobox");

    // The regression: these used to be disabled behind the Enabled checkbox.
    expect(textarea).toBeEnabled();
    expect(select).toBeEnabled();
  });

  it("enables the prompt automatically as you type and saves it", async () => {
    const user = userEvent.setup();
    const { onSaved } = renderModal(null);

    await user.click(screen.getByRole("button", { name: /prompt/i }));
    const textarea = await screen.findByPlaceholderText(/this project uses pnpm/i);
    await user.type(textarea, "Always run pnpm test.");

    // Typing turns the (previously off) prompt on.
    const enabled = screen.getByRole("checkbox") as HTMLInputElement;
    expect(enabled.checked).toBe(true);

    await user.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(apiMock.updateProjectConfig).toHaveBeenCalled());
    const [, config] = apiMock.updateProjectConfig.mock.calls[0];
    expect(config.system_prompt).toMatchObject({
      text: "Always run pnpm test.",
      position: "prefix",
      enabled: true,
    });
    expect(onSaved).toHaveBeenCalled();
  });

  it("changing the position enables and saves the prompt", async () => {
    const user = userEvent.setup();
    renderModal(null);

    await user.click(screen.getByRole("button", { name: /prompt/i }));
    const select = await screen.findByRole("combobox");
    await user.selectOptions(select, "suffix");
    await user.type(
      await screen.findByPlaceholderText(/this project uses pnpm/i),
      "Be terse.",
    );
    await user.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(apiMock.updateProjectConfig).toHaveBeenCalled());
    const [, config] = apiMock.updateProjectConfig.mock.calls[0];
    expect(config.system_prompt).toMatchObject({
      text: "Be terse.",
      position: "suffix",
      enabled: true,
    });
  });

  it("unchecking Enabled keeps the text but stops applying it", async () => {
    const user = userEvent.setup();
    renderModal({ position: "prefix", text: "Keep me", enabled: true });

    await user.click(screen.getByRole("button", { name: /prompt/i }));
    const checkbox = await screen.findByRole("checkbox");
    await user.click(checkbox);
    expect((checkbox as HTMLInputElement).checked).toBe(false);

    await user.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() => expect(apiMock.updateProjectConfig).toHaveBeenCalled());
    const [, config] = apiMock.updateProjectConfig.mock.calls[0];
    expect(config.system_prompt).toMatchObject({ text: "Keep me", enabled: false });
  });
});
