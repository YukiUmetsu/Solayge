import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { PullToRefresh } from "./PullToRefresh";

function setup(onRefresh: () => Promise<void>) {
  const { container } = render(
    <PullToRefresh onRefresh={onRefresh}>
      <p>content</p>
    </PullToRefresh>,
  );
  const ptr = container.querySelector(".ptr") as HTMLElement;
  expect(ptr).not.toBeNull();
  return ptr;
}

describe("PullToRefresh", () => {
  it("refreshes when released past the threshold and shows the progression", async () => {
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const ptr = setup(onRefresh);

    fireEvent.touchStart(ptr, { touches: [{ clientY: 0 }] });
    fireEvent.touchMove(ptr, { touches: [{ clientY: 200 }] });
    expect(screen.getByText("Release to refresh")).toBeInTheDocument();

    fireEvent.touchEnd(ptr);
    expect(screen.getByText("Refreshing…")).toBeInTheDocument();

    await waitFor(() => expect(onRefresh).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Updated")).toBeInTheDocument();
  });

  it("does not refresh when released below the threshold", () => {
    const onRefresh = vi.fn();
    const ptr = setup(onRefresh);

    fireEvent.touchStart(ptr, { touches: [{ clientY: 0 }] });
    fireEvent.touchMove(ptr, { touches: [{ clientY: 40 }] });
    expect(screen.getByText("Pull to refresh")).toBeInTheDocument();

    fireEvent.touchEnd(ptr);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("stays disarmed when the list is not scrolled to the top", () => {
    const onRefresh = vi.fn();
    const ptr = setup(onRefresh);
    Object.defineProperty(ptr, "scrollTop", { value: 100, writable: true });

    fireEvent.touchStart(ptr, { touches: [{ clientY: 0 }] });
    fireEvent.touchMove(ptr, { touches: [{ clientY: 200 }] });
    fireEvent.touchEnd(ptr);

    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("still resets when the refresh itself fails", async () => {
    const onRefresh = vi.fn().mockRejectedValue(new Error("offline"));
    const ptr = setup(onRefresh);

    fireEvent.touchStart(ptr, { touches: [{ clientY: 0 }] });
    fireEvent.touchMove(ptr, { touches: [{ clientY: 200 }] });
    fireEvent.touchEnd(ptr);

    await waitFor(() => expect(onRefresh).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("Updated")).toBeInTheDocument();
  });
});
