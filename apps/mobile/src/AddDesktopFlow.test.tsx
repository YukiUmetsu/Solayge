import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { pairingApiMock } = vi.hoisted(() => ({
  pairingApiMock: {
    parsePairingQr: vi.fn(),
    pairWithQr: vi.fn(),
    pairPoll: vi.fn(),
    pairCancel: vi.fn(),
    pairedDesktops: vi.fn(),
    forgetDesktop: vi.fn(),
    mobileIdentity: vi.fn(),
    manualPairingRequest: vi.fn(),
  },
}));

vi.mock("./pairing", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./pairing")>();
  return { ...actual, pairingApi: pairingApiMock };
});

import { AddDesktopFlow } from "./AddDesktopFlow";

const FUTURE = Math.floor(Date.now() / 1000) + 120;

beforeEach(() => {
  pairingApiMock.parsePairingQr.mockReset().mockResolvedValue({
    desktop_id: "did_desk",
    desktop_public_key: "key",
    desktop_fingerprint: "AB12 CD34",
    pairing_id: "pair_1",
    expires_at: FUTURE,
    verification_code: "123456",
    device_id: "did_phone",
    reachable: true,
  });
  pairingApiMock.pairWithQr.mockReset().mockResolvedValue({
    pairing_id: "pair_1",
    device_id: "did_phone",
    desktop_id: "did_desk",
    verification_code: "123456",
  });
  pairingApiMock.pairPoll.mockReset().mockResolvedValue({ state: "pending" });
  pairingApiMock.pairCancel.mockReset().mockResolvedValue(undefined);
});

describe("Add a desktop wizard", () => {
  it("walks from preparation to verification and shows the shared code", async () => {
    const user = userEvent.setup();
    render(<AddDesktopFlow onClose={() => {}} />);

    await user.click(screen.getByRole("button", { name: /get started/i }));
    expect(screen.getByText(/Settings → Devices/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /paste pairing link/i }));
    await user.click(screen.getByLabelText("Paste pairing link"));
    await user.paste('{"protocol_version":1}');
    await user.click(screen.getByRole("button", { name: /^continue$/i }));

    expect(await screen.findByText("123 456")).toBeInTheDocument();
    expect(pairingApiMock.pairWithQr).toHaveBeenCalled();
  });

  it("explains when the code carries no connection address", async () => {
    const user = userEvent.setup();
    pairingApiMock.parsePairingQr.mockResolvedValue({
      desktop_id: "did_desk",
      desktop_public_key: "key",
      desktop_fingerprint: "AB12",
      pairing_id: "pair_1",
      expires_at: FUTURE,
      verification_code: "123456",
      device_id: "did_phone",
      reachable: false,
    });
    render(<AddDesktopFlow onClose={() => {}} />);

    await user.click(screen.getByRole("button", { name: /get started/i }));
    await user.click(screen.getByRole("button", { name: /paste pairing link/i }));
    await user.click(screen.getByLabelText("Paste pairing link"));
    await user.paste("{}");
    await user.click(screen.getByRole("button", { name: /^continue$/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/no connection address/i);
    expect(pairingApiMock.pairWithQr).not.toHaveBeenCalled();
  });

  it("celebrates a finished pairing and hands the desktop back", async () => {
    const user = userEvent.setup();
    const onPaired = vi.fn();
    const onClose = vi.fn();
    pairingApiMock.pairPoll.mockResolvedValue({
      state: "approved",
      desktop: {
        id: "did_desk",
        name: "MacBook Pro",
        platform: "macos",
        fingerprint: "AB12 CD34",
      },
    });
    render(<AddDesktopFlow onClose={onClose} onPaired={onPaired} />);

    await user.click(screen.getByRole("button", { name: /get started/i }));
    await user.click(screen.getByRole("button", { name: /paste pairing link/i }));
    await user.click(screen.getByLabelText("Paste pairing link"));
    await user.paste("{}");
    await user.click(screen.getByRole("button", { name: /^continue$/i }));

    expect(await screen.findByText(/connected!/i)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /start using solayge/i }));
    expect(onPaired).toHaveBeenCalledWith(
      expect.objectContaining({ id: "did_desk", name: "MacBook Pro" }),
    );
    expect(onClose).toHaveBeenCalled();
  });
});
