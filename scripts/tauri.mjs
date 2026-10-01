#!/usr/bin/env node
// Tauri CLI wrapper for Solayge.
//
// `tauri build` on macOS builds the .dmg with create-dmg's `bundle_dmg.sh`,
// which drives `hdiutil` and Finder via AppleScript. Those steps fail
// intermittently (hdiutil "Resource busy", AppleEvent -1728/-1743), and a
// failure leaves a read-write `rw.*.dmg` staging image — and sometimes a mounted
// volume — behind, which can make the next run fail too. This wrapper is
// transparent for every other command (`dev`, `icon`, …): for `build` on macOS
// it clears that stale state before bundling and retries a DMG-specific failure
// so a transient hiccup does not fail the whole release.
//
// It is wired up as the package's `tauri` script, so `pnpm tauri build` uses it.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const args = process.argv.slice(2);
const isMac = process.platform === "darwin";
const isBuild = args[0] === "build";
/** Total attempts for `build` (the first run plus retries). */
const MAX_ATTEMPTS = 3;

const bundleMacDir = join(root, "src-tauri", "target", "release", "bundle", "macos");

/** Resolve the real Tauri CLI binary, falling back to `PATH`. */
function tauriBin() {
  const name = process.platform === "win32" ? "tauri.cmd" : "tauri";
  const local = join(root, "node_modules", ".bin", name);
  return existsSync(local) ? local : name;
}

/**
 * Detach volumes backed by one of our own staging images and delete leftover
 * staging/final `.dmg` files from the macOS bundle directory. Only images whose
 * `image-path` is inside our bundle directory are touched, so unrelated images
 * the user happens to have mounted are left alone.
 */
function cleanStaleDmgState() {
  if (!isMac) return;

  const info = spawnSync("hdiutil", ["info"], { encoding: "utf8" }).stdout ?? "";
  // Blocks are separated by lines of `=`; each carries `image-path` plus the
  // mounted device lines.
  for (const block of info.split(/^=+\s*$/m)) {
    const image = block.match(/image-path\s*:\s*(.+)/)?.[1]?.trim();
    if (!image || !image.startsWith(bundleMacDir)) continue;
    for (const line of block.split("\n")) {
      // e.g. `/dev/disk4s1  Apple_HFS  /Volumes/dmg.AbC123`
      const device = line.match(/^(\/dev\/disk\S+)\s+\S+\s+\S/)?.[1];
      if (!device) continue;
      console.log(`[bundle] detaching stale staging volume ${device} (${image})`);
      spawnSync("hdiutil", ["detach", device, "-force"], { stdio: "ignore" });
    }
  }

  if (!existsSync(bundleMacDir)) return;
  for (const name of readdirSync(bundleMacDir)) {
    // `rw.<pid>.<dmg-name>` is the staging image; a bare `*.dmg` here is an
    // intermediate the Tauri bundler renames into `bundle/dmg/` on success, so
    // anything left is from a failed run and would break `hdiutil convert`.
    if (/^rw\..*\.dmg$/.test(name) || /\.dmg$/.test(name)) {
      console.log(`[bundle] removing stale disk image ${name}`);
      rmSync(join(bundleMacDir, name), { force: true });
    }
  }
}

/** Run the Tauri CLI, streaming its output live while also capturing it. */
function runTauri() {
  return new Promise((resolve) => {
    const child = spawn(tauriBin(), args, {
      cwd: root,
      env: process.env,
      stdio: ["inherit", "pipe", "pipe"],
      shell: process.platform === "win32",
    });
    let output = "";
    child.stdout.on("data", (chunk) => {
      output += chunk.toString();
      process.stdout.write(chunk);
    });
    child.stderr.on("data", (chunk) => {
      output += chunk.toString();
      process.stderr.write(chunk);
    });
    child.on("error", (e) => resolve({ code: 1, output: `${output}\n${e.message}` }));
    child.on("close", (code) => resolve({ code: code ?? 1, output }));
  });
}

/** Pass a non-build (or non-macOS) command straight through. */
function passthrough() {
  const child = spawn(tauriBin(), args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  child.on("close", (code) => process.exit(code ?? 1));
  child.on("error", (e) => {
    console.error(e.message);
    process.exit(1);
  });
}

async function main() {
  if (!isMac || !isBuild) {
    passthrough();
    return;
  }

  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    cleanStaleDmgState();
    const { code, output } = await runTauri();
    if (code === 0) process.exit(0);

    // Only retry the flaky disk-image stage; a real compile/config error should
    // surface immediately instead of being run again.
    const dmgRelated = /bundle_dmg|hdiutil|create-dmg|osascript/i.test(output);
    if (attempt < MAX_ATTEMPTS && dmgRelated) {
      console.error(
        `\n[bundle] macOS DMG step failed (attempt ${attempt}/${MAX_ATTEMPTS}); ` +
          `clearing stale state and retrying…\n`,
      );
      continue;
    }
    if (dmgRelated) {
      console.error(
        "\n[bundle] The macOS DMG step kept failing. Re-run with " +
          "`pnpm tauri build --bundles dmg -v` to see the full bundle_dmg.sh output. " +
          "If it mentions an AppleEvent error (-1728/-1743), allow your terminal to " +
          "control Finder under System Settings → Privacy & Security → Automation.\n",
      );
    }
    process.exit(code);
  }
}

main();
