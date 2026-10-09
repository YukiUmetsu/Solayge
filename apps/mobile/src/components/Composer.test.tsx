import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { Composer, type SendState } from "./Composer";

function setup(sendState: SendState | null = null, overrides: Partial<Parameters<typeof Composer>[0]> = {}) {
  const onSend = vi.fn();
  const onStop = vi.fn();
  const onRetry = vi.fn();
  render(
    <Composer
      onSend={onSend}
      onStop={onStop}
      onRetry={onRetry}
      sendState={sendState}
      canStop={false}
      readOnly={false}
      {...overrides}
    />,
  );
  return { onSend, onStop, onRetry };
}

describe("Composer", () => {
  it("sends trimmed text and clears the field", async () => {
    const user = userEvent.setup();
    const { onSend } = setup();
    const input = screen.getByLabelText("Message your agent");
    await user.type(input, "  run the tests  ");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(onSend).toHaveBeenCalledWith("run the tests");
    expect(input).toHaveValue("");
  });

  it("shows the four delivery stages", () => {
    const { rerender } = render(
      <Composer
        onSend={vi.fn()}
        onStop={vi.fn()}
        onRetry={vi.fn()}
        sendState={{ status: "delivered", errorCode: null, errorMessage: null }}
        canStop={false}
        readOnly={false}
      />,
    );
    expect(screen.getByText("Received by desktop")).toBeInTheDocument();

    rerender(
      <Composer
        onSend={vi.fn()}
        onStop={vi.fn()}
        onRetry={vi.fn()}
        sendState={{ status: "succeeded", errorCode: null, errorMessage: null }}
        canStop={false}
        readOnly={false}
      />,
    );
    expect(screen.getByText("Completed")).toBeInTheDocument();
  });

  it("offers Tap to retry after a definite pre-dispatch rejection", async () => {
    const user = userEvent.setup();
    const { onRetry } = setup({ status: "failed", errorCode: "not_found", errorMessage: "gone" });
    const retry = screen.getByRole("button", { name: "Tap to retry" });
    await user.click(retry);
    expect(onRetry).toHaveBeenCalled();
  });

  it("never offers retry for an ambiguous transport outcome", () => {
    setup({ status: "unknown", errorCode: "transport", errorMessage: "timeout" });
    expect(screen.getByText("Delivery unknown")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Tap to retry" })).toBeNull();
    expect(screen.getByText(/checking with the desktop/i)).toBeInTheDocument();
  });

  it("keeps Stop visually available only when supported", async () => {
    const user = userEvent.setup();
    const { onStop } = setup(null, { canStop: true });
    await user.click(screen.getByRole("button", { name: "Stop task" }));
    expect(onStop).toHaveBeenCalled();
  });

  it("disables messaging and stopping when read-only", () => {
    setup(null, { readOnly: true, disabledReason: "You're offline." });
    expect(screen.getByLabelText("Message your agent")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop task" })).toBeDisabled();
    expect(screen.getByText("You're offline.")).toBeInTheDocument();
  });
});
