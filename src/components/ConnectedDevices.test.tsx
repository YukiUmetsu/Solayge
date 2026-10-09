import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    devicesList: vi.fn(),
    pairingStart: vi.fn(),
    pairingStatus: vi.fn(),
    pairingApprove: vi.fn(),
    pairingReject: vi.fn(),
    deviceRevoke: vi.fn(),
  },
}));
vi.mock("../api", () => ({ api: apiMock }));

import { ConnectedDevices } from "./ConnectedDevices";
import type { ConnectedDevice, PairingOffer } from "../types";

const PHONE: ConnectedDevice = {
  device_id: "did_phone",
  name: "Pixel 9",
  platform: "android",
  scopes: ["sessions.read", "sessions.message"],
  paired_at: 1_700_000_000,
  last_seen: 1_700_003_600,
  online: true,
  fingerprint: "AB12 CD34 EF56 7890",
};

const OFFER: PairingOffer = {
  pairing_id: "pair_1",
  manual_code: "ABCD-EFGH",
  expires_at: 1_700_000_120,
  qr_string: '{"protocol_version":1,"pairing_id":"pair_1"}',
  desktop_id: "did_desktop",
  desktop_fingerprint: "1234 5678 9ABC DEF0",
  reachable: true,
};

beforeEach(() => {
  apiMock.devicesList.mockReset().mockResolvedValue([PHONE]);
  apiMock.pairingStart.mockReset().mockResolvedValue(OFFER);
  apiMock.pairingStatus.mockReset().mockResolvedValue({ state: "pending" });
  apiMock.pairingApprove.mockReset().mockResolvedValue(PHONE);
  apiMock.pairingReject.mockReset().mockResolvedValue(undefined);
  apiMock.deviceRevoke.mockReset().mockResolvedValue(undefined);
});

describe("Connected devices", () => {
  it("lists name, platform, scopes, dates, and online status", async () => {
    render(<ConnectedDevices />);

    const name = await screen.findByText("Pixel 9");
    expect(name).toBeInTheDocument();
    expect(screen.getByText("android")).toBeInTheDocument();
    expect(screen.getByText("View · Message")).toBeInTheDocument();
    expect(screen.getByText(/Paired /)).toBeInTheDocument();
    expect(screen.getByText(/Last seen /)).toBeInTheDocument();
    expect(screen.getByText("Online")).toBeInTheDocument();
  });

  it("revokes only after an explicit confirmation and refreshes", async () => {
    const user = userEvent.setup();
    apiMock.devicesList
      .mockResolvedValueOnce([PHONE])
      .mockResolvedValueOnce([]);
    render(<ConnectedDevices />);

    await user.click(await screen.findByRole("button", { name: /revoke access/i }));
    expect(apiMock.deviceRevoke).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /^revoke$/i }));

    await waitFor(() =>
      expect(apiMock.deviceRevoke).toHaveBeenCalledWith("did_phone"),
    );
    expect(await screen.findByText(/No phones are connected/i)).toBeInTheDocument();
  });

  it("shows a QR and manual code, and approves with the default View + Message grant", async () => {
    const user = userEvent.setup();
    render(<ConnectedDevices />);

    await user.click(await screen.findByRole("button", { name: /add phone/i }));

    expect(await screen.findByText("ABCD-EFGH")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /pairing qr code/i })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /approve phone/i }));

    await waitFor(() =>
      expect(apiMock.pairingApprove).toHaveBeenCalledWith("pair_1", [
        "sessions.read",
        "sessions.message",
      ]),
    );
  });

  it("shows the verification code and phone details once pairing begins", async () => {
    const user = userEvent.setup();
    apiMock.pairingStatus.mockResolvedValue({
      state: "pending",
      verification_code: "123456",
      device_name: "Pixel 9",
      device_platform: "android",
    });
    render(<ConnectedDevices />);
    await user.click(await screen.findByRole("button", { name: /add phone/i }));

    expect(await screen.findByText("123 456")).toBeInTheDocument();
    expect(screen.getByText(/Pixel 9 · android/)).toBeInTheDocument();
  });

  it("never offers device administration as a pairing scope", async () => {
    const user = userEvent.setup();
    render(<ConnectedDevices />);
    await user.click(await screen.findByRole("button", { name: /add phone/i }));

    await screen.findByRole("checkbox", { name: "View" });
    expect(screen.queryByRole("checkbox", { name: /manage devices/i })).toBeNull();
    expect(screen.queryByText(/manage devices/i)).toBeNull();
  });

  it("sends only the checked scopes when one is removed", async () => {
    const user = userEvent.setup();
    render(<ConnectedDevices />);
    await user.click(await screen.findByRole("button", { name: /add phone/i }));

    await user.click(await screen.findByRole("checkbox", { name: "Message" }));
    await user.click(screen.getByRole("button", { name: /approve phone/i }));

    await waitFor(() =>
      expect(apiMock.pairingApprove).toHaveBeenCalledWith("pair_1", [
        "sessions.read",
      ]),
    );
  });
});
