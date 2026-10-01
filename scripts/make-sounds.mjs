#!/usr/bin/env node
// Generate the bundled notification sounds under `public/sounds`.
//
// Solayge ships four short tones — task complete, task failed, entered review,
// and needs-attention. They are synthesized here (pure sine notes with an
// exponential decay) so the repo carries no third-party audio and the tones can
// be re-tuned by editing this file. Run with `node scripts/make-sounds.mjs`.
//
// Each event is deliberately distinct in pitch, rhythm and timbre:
//   complete  — a rising major arpeggio that resolves up (satisfying "done").
//   review    — a single soft ping ("something settled").
//   attention — three sharp repeated beeps ("come look").
//   failed    — a rough descending two-tone buzz ("went wrong").

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SAMPLE_RATE = 44100;
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const outDir = join(root, "public", "sounds");

// Equal-tempered note frequencies (Hz) used below, so the tones stay in tune.
const C4 = 261.63;
const C5 = 523.25;
const E5 = 659.25;
const F5 = 698.46;
const G5 = 783.99;
const A5 = 880.0;
const AS4 = 466.16;
const B5 = 987.77;
const C6 = 1046.5;
const E6 = 1318.51;

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

/**
 * Add a harsher, square-ish note (odd harmonics) for the "something is wrong"
 * alarm. The extra partials give it a rough edge that reads as an alert rather
 * than a musical chime.
 */
function buzz(buf, freq, at, dur, gain = 0.55) {
  const start = Math.floor(at * SAMPLE_RATE);
  const len = Math.floor(dur * SAMPLE_RATE);
  const attack = Math.floor(0.004 * SAMPLE_RATE);
  for (let i = 0; i < len; i++) {
    const idx = start + i;
    if (idx >= buf.length) break;
    const t = i / SAMPLE_RATE;
    const env = i < attack ? i / attack : Math.exp(-2.6 * (t / dur));
    let wave = 0;
    for (let h = 1; h <= 7; h += 2) {
      wave += Math.sin(2 * Math.PI * freq * h * t) / h;
    }
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

const seconds = 0.68;
const make = () => new Float32Array(Math.floor(seconds * SAMPLE_RATE));

const sounds = {
  // A warm C-major arpeggio resolving up to a ringing high C: unmistakably "done".
  complete: () => {
    const b = make();
    note(b, C4, 0.0, 0.6, 0.32); // low root for body
    note(b, C5, 0.0, 0.16, 0.5);
    note(b, E5, 0.11, 0.16, 0.5);
    note(b, G5, 0.22, 0.18, 0.5);
    note(b, C6, 0.33, 0.34, 0.6); // ringing resolution
    note(b, E6, 0.33, 0.34, 0.18); // sparkle on top
    return b;
  },
  // A single soft ping: neutral, unobtrusive feedback.
  review: () => {
    const b = make();
    note(b, A5, 0.0, 0.26, 0.5);
    note(b, E6, 0.0, 0.14, 0.12); // faint shimmer
    return b;
  },
  // Three equal, insistent beeps — a rhythm that asks to be looked at.
  attention: () => {
    const b = make();
    note(b, B5, 0.0, 0.1, 0.6);
    note(b, B5, 0.17, 0.1, 0.6);
    note(b, B5, 0.34, 0.1, 0.6);
    return b;
  },
  // A rough two-tone drop (high then low) with a buzzy edge: an error alert.
  failed: () => {
    const b = make();
    buzz(b, F5, 0.0, 0.14, 0.5);
    buzz(b, AS4, 0.14, 0.48, 0.52);
    return b;
  },
};

mkdirSync(outDir, { recursive: true });
for (const [name, render] of Object.entries(sounds)) {
  const path = join(outDir, `${name}.wav`);
  writeFileSync(path, toWav(render()));
  console.log(`wrote ${path}`);
}
