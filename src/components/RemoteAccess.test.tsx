import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock } = vi.hoisted(() => ({
  apiMock: {
    remoteStatus: vi.fn(),
    remoteApply: vi.fn(),
    remoteRetry: vi.fn(),
    pushTestQuestion: vi.fn(),
    pushDevConfig: vi.fn(),
    pushDevConfigSet: vi.fn(),
    pushDevConfigClear: vi.fn(),
    pushGatewayHealth: vi.fn(),
  },
}));
vi.mock("../api", () => ({ api: apiMock }));

import { RemoteAccess } from "./RemoteAccess";
import type { RemoteStatus } from "../types";

const STATUS: RemoteStatus = {
  enabled: true,
  state: "active",
  listener_running: true,
  listener_port: 51234,
  connected_clients: 2,
  tailscale_backup: true,
  tailscale: {
    installed: true,
    backend_state: "Running",
    authenticated: true,
    online: true,
    dns_name: "macbook-pro.berylline-goblin.ts.net",
    version: "1.58.2",
  },
  serve_enabled: true,
  serve_owned: true,
  serve_url: "https://macbook-pro.berylline-goblin.ts.net/",
  serve_port: 443,
  https_port: 443,
  local_port: 51234,
};

beforeEach(() => {
  apiMock.remoteStatus.mockReset().mockResolvedValue(STATUS);
  apiMock.remoteApply.mockReset().mockResolvedValue(STATUS);
  apiMock.remoteRetry.mockReset().mockResolvedValue(STATUS);
  apiMock.pushTestQuestion.mockReset();
  apiMock.pushDevConfig.mockReset().mockResolvedValue({
    url: "",
    has_token: false,
    source: "none",
  });
  apiMock.pushDevConfigSet.mockReset();
  apiMock.pushDevConfigClear.mockReset();
  apiMock.pushGatewayHealth.mockReset();
});

describe("Remote access", () => {
  it("shows the listener, client count, Tailscale state, and Serve endpoint", async () => {
    render(<RemoteAccess />);

    expect(await screen.findByText("Running on 127.0.0.1:51234")).toBeInTheDocument();
    expect(screen.getByText("2 phones")).toBeInTheDocument();
    expect(
      screen.getByText(/Running · macbook-pro\.berylline-goblin\.ts\.net/),
    ).toBeInTheDocument();
    expect(
      screen.getByText("https://macbook-pro.berylline-goblin.ts.net/"),
    ).toBeInTheDocument();
  });

  it("enabling mobile connections applies the persisted preference", async () => {
    const user = userEvent.setup();
    apiMock.remoteStatus.mockResolvedValue({ ...STATUS, enabled: false, state: "disabled" });
    render(<RemoteAccess />);

    const toggle = await screen.findByRole("checkbox", {
      name: /enable mobile connections/i,
    });
    await user.click(toggle);

    await waitFor(() =>
      expect(apiMock.remoteApply).toHaveBeenCalledWith({
        enabled: true,
        tailscale_backup: true,
        https_port: 443,
        local_port: 51234,
      }),
    );
  });

  it("suggests installing Tailscale when it is missing", async () => {
    apiMock.remoteStatus.mockResolvedValue({
      ...STATUS,
      state: "degraded",
      serve_owned: false,
      serve_url: undefined,
      tailscale: {
        installed: false,
        backend_state: "NotInstalled",
        authenticated: false,
        online: false,
        message: "Tailscale is not installed",
      },
      last_error: "Tailscale is not installed",
    });
    render(<RemoteAccess />);

    expect(await screen.findByText("Needs attention")).toBeInTheDocument();
    expect(screen.getByText(/Install Tailscale from tailscale\.com/i)).toBeInTheDocument();
  });

  it("the dev test button sends a synthetic question push and shows the result", async () => {
    const user = userEvent.setup();
    apiMock.pushTestQuestion.mockResolvedValue(
      "Sent a test question push to 1 device(s).",
    );
    render(<RemoteAccess />);
    await screen.findByText("Running on 127.0.0.1:51234");

    await user.type(screen.getByPlaceholderText("ses_…"), "ses_42");
    await user.click(
      screen.getByRole("button", { name: /send test question push/i }),
    );

    await waitFor(() =>
      expect(apiMock.pushTestQuestion).toHaveBeenCalledWith("ses_42"),
    );
    expect(
      await screen.findByText(/Sent a test question push to 1 device/),
    ).toBeInTheDocument();
  });

  it("the dev panel saves the gateway override", async () => {
    const user = userEvent.setup();
    apiMock.pushDevConfigSet.mockResolvedValue({
      url: "http://127.0.0.1:8787/v1/push",
      has_token: true,
      source: "dev",
    });
    render(<RemoteAccess />);
    await screen.findByText("Running on 127.0.0.1:51234");

    await user.type(
      screen.getByPlaceholderText("http://127.0.0.1:8787/v1/push"),
      "http://127.0.0.1:8787/v1/push",
    );
    await user.type(screen.getByPlaceholderText("PUSH_GATEWAY_TOKEN"), "secret");
    await user.click(screen.getByRole("button", { name: /save gateway/i }));

    await waitFor(() =>
      expect(apiMock.pushDevConfigSet).toHaveBeenCalledWith(
        "http://127.0.0.1:8787/v1/push",
        "secret",
      ),
    );
    expect(await screen.findByText(/Gateway saved/)).toBeInTheDocument();
  });

  it("retry re-runs the lifecycle check", async () => {
    const user = userEvent.setup();
    render(<RemoteAccess />);
    await screen.findByText("Running on 127.0.0.1:51234");

    await user.click(screen.getByRole("button", { name: /retry & re-check/i }));
    await waitFor(() => expect(apiMock.remoteRetry).toHaveBeenCalledTimes(1));
  });

  it("renders a build without a Tailscale status without crashing", async () => {
    // A `--no-default-features` desktop reports only a subset of the status
    // (no `tailscale` object); the panel must still render.
    apiMock.remoteStatus.mockResolvedValue({
      enabled: false,
      state: "unavailable",
      listener_running: false,
      listener_port: null,
      connected_clients: 0,
      tailscale_backup: false,
      serve_enabled: false,
      serve_owned: false,
      https_port: 443,
      local_port: 51234,
      last_error: "Remote access is unavailable in this build",
    } as unknown as RemoteStatus);
    render(<RemoteAccess />);

    expect(await screen.findByText("Needs attention")).toBeInTheDocument();
    expect(screen.getByText("Not installed")).toBeInTheDocument();
  });
});
