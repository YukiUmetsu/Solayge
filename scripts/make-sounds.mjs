#!/usr/bin/env node
// Generate the bundled notification sounds under `public/sounds`.
//
// Solayge ships four short tones — task complete, task failed, entered review,
// and needs-attention. They are synthesized here (pure sine notes with an
// exponential decay) so the repo carries no third-party audio and the tones can
// be re-tuned by editing this file. Run with `node scripts/make-sounds.mjs`.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SAMPLE_RATE = 44100;
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outDir = join(root, "public", "sounds");

/** Add a decaying sine note into `buf` starting at `at` seconds. */
function note(buf, freq, at, dur, gain = 0.7) {
  const start = Math.floor(at * SAMPLE_RATE);
  const len = Math.floor(dur * SAMPLE_RATE);
  const attack = Math.floor(0.006 * SAMPLE_RATE);
  for (let i = 0; i < len; i++) {
    const idx = start + i;
    if (idx >= buf.length) break;
    const t = i / SAMPLE_RATE;
    // Short linear attack, then an exponential tail, so notes don't click.
    const env = i < attack ? i / attack : Math.exp(-4.5 * (t / dur));
    // A couple of quiet harmonics make the tone less like a raw sine.
    const wave =
      Math.sin(2 * Math.PI * freq * t) +
      0.22 * Math.sin(4 * Math.PI * freq * t) +
      0.07 * Math.sin(6 * Math.PI * freq * t);
    buf[idx] += wave * env * gain;
  }
}

/** Encode a Float32 buffer (-1..1) as 16-bit mono PCM WAV bytes. */
function toWav(buf) {
  const data = Buffer.alloc(buf.length * 2);
  let peak = 0;
  for (const s of buf) peak = Math.max(peak, Math.abs(s));
  const norm = peak > 1 ? 1 / peak : 1;
  for (let i = 0; i < buf.length; i++) {
    const v = Math.max(-1, Math.min(1, buf[i] * norm));
    data.writeInt16LE(Math.round(v * 32767), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20); // PCM
  header.writeUInt16LE(1, 22); // mono
  header.writeUInt32LE(SAMPLE_RATE, 24);
  header.writeUInt32LE(SAMPLE_RATE * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

const seconds = 0.62;
const make = () => new Float32Array(Math.floor(seconds * SAMPLE_RATE));

const sounds = {
  // A bright rising triad: unmistakably "done".
  complete: () => {
    const b = make();
    note(b, 523.25, 0.0, 0.16); // C5
    note(b, 659.25, 0.13, 0.16); // E5
    note(b, 783.99, 0.26, 0.34); // G5
    return b;
  },
  // A soft descending pair: "something went wrong".
  failed: () => {
    const b = make();
    note(b, 392.0, 0.0, 0.2, 0.6); // G4
    note(b, 261.63, 0.16, 0.42, 0.6); // C4
    return b;
  },
  // A single gentle ping for entering review.
  review: () => {
    const b = make();
    note(b, 1174.66, 0.0, 0.1, 0.5); // D6
    note(b, 880.0, 0.09, 0.3, 0.5); // A5
    return b;
  },
  // A double beep that reads as "needs you".
  attention: () => {
    const b = make();
    note(b, 659.25, 0.0, 0.12); // E5
    note(b, 659.25, 0.2, 0.12); // E5
    return b;
  },
};

mkdirSync(outDir, { recursive: true });
for (const [name, render] of Object.entries(sounds)) {
  const path = join(outDir, `${name}.wav`);
  writeFileSync(path, toWav(render()));
  console.log(`wrote ${path}`);
}
