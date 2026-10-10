#!/usr/bin/env node
// Dev helper: start the Solayge push gateway locally and point the desktop app
// at it, so a background push to the phone can be tested in a few clicks.
//
// It:
//   1. finds the Firebase service-account JSON,
//   2. reuses (or generates) a gateway bearer token,
//   3. writes the desktop's *dev-only* gateway config (`push-dev.json`, 0600) in
//      the app data directory so no environment variables or restart are needed,
//   4. runs `cargo run -p solayge-push-gateway`.
//
// Then: Solayge → Settings → Remote access → "Send test question push".
//
// Usage:
//   pnpm push:gateway
//   GOOGLE_APPLICATION_CREDENTIALS=/path/to/sa.json pnpm push:gateway
//   pnpm push:gateway --service-account /path/to/sa.json --port 8787

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, chmodSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const APP_ID = "com.solayge.desktop";
const DEV_CONFIG_FILE = "push-dev.json";

function parseArgs(argv) {
  const args = { port: 8787, serviceAccount: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--service-account" || arg === "-s") args.serviceAccount = argv[++i];
    else if (arg === "--port" || arg === "-p") args.port = Number(argv[++i]);
    else if (arg === "--help" || arg === "-h") {
      console.log(
        [
          "Usage: pnpm push:gateway [options]",
          "",
          "  -s, --service-account <path>  Firebase service-account JSON",
          "  -p, --port <port>             gateway port (default 8787)",
          "  -h, --help                    show this help",
        ].join("\n"),
      );
      process.exit(0);
    }
  }
  return args;
}

/** Tauri's `app_data_dir()` for this app identifier, per platform. */
function appDataDir() {
  if (process.platform === "darwin") {
    return join(homedir(), "Library", "Application Support", APP_ID);
  }
  if (process.platform === "win32") {
    const base = process.env.APPDATA ?? join(homedir(), "AppData", "Roaming");
    return join(base, APP_ID);
  }
  const base = process.env.XDG_DATA_HOME ?? join(homedir(), ".local", "share");
  return join(base, APP_ID);
}

function firstExisting(paths) {
  return paths.find((path) => path && existsSync(path)) ?? null;
}

function missingServiceAccount() {
  console.error(
    [
      "",
      "Missing the Firebase service-account JSON (it is not the same as google-services.json).",
      "",
      "Create one:",
      "  1. Firebase console → Project settings → Service accounts",
      "  2. Generate new private key, save it somewhere safe, e.g.",
      "     ~/.solayge/fcm-service-account.json",
      "  3. Re-run with the path, e.g.",
      "     GOOGLE_APPLICATION_CREDENTIALS=~/.solayge/fcm-service-account.json pnpm push:gateway",
      "",
    ].join("\n"),
  );
  process.exit(1);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const port = Number.isInteger(args.port) && args.port > 0 ? args.port : 8787;

  const serviceAccount = firstExisting([
    args.serviceAccount,
    process.env.GOOGLE_APPLICATION_CREDENTIALS,
    join(homedir(), ".solayge", "fcm-service-account.json"),
  ]);
  if (!serviceAccount) missingServiceAccount();

  const dataDir = appDataDir();
  const configPath = join(dataDir, DEV_CONFIG_FILE);

  // Reuse the token from a previous run so the desktop/app do not churn.
  let token = process.env.PUSH_GATEWAY_TOKEN?.trim();
  if (!token && existsSync(configPath)) {
    try {
      const existing = JSON.parse(readFileSync(configPath, "utf8"));
      if (typeof existing.token === "string" && existing.token.length > 0) {
        token = existing.token;
      }
    } catch {
      // Corrupt config; fall through and regenerate.
    }
  }
  if (!token) token = randomBytes(32).toString("hex");

  const url = `http://127.0.0.1:${port}/v1/push`;
  mkdirSync(dataDir, { recursive: true });
  writeFileSync(configPath, JSON.stringify({ url, token }, null, 2));
  try {
    chmodSync(configPath, 0o600);
  } catch {
    // Best effort; Windows has no POSIX mode.
  }

  console.log("");
  console.log("Solayge push gateway");
  console.log(`  service account : ${serviceAccount}`);
  console.log(`  gateway URL     : ${url}`);
  console.log(`  desktop config  : ${configPath}`);
  console.log("");
  console.log("In Solayge → Settings → Remote access → Developer: test background push,");
  console.log("leave the fields as pre-filled and click \"Send test question push\".");
  console.log("");

  const child = spawn("cargo", ["run", "-p", "solayge-push-gateway"], {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      PUSH_GATEWAY_TOKEN: token,
      GOOGLE_APPLICATION_CREDENTIALS: serviceAccount,
      PUSH_GATEWAY_BIND: `127.0.0.1:${port}`,
    },
  });
  child.on("error", (error) => {
    console.error(`Could not start cargo: ${error.message}`);
    process.exit(1);
  });
  child.on("close", (code) => process.exit(code ?? 0));
}

main();
