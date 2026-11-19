/*
rng.mjs - seedable random numbers for sound generation
Copyright (C) 2026 Strudel contributors - see <https://codeberg.org/uzu/strudel/src/branch/main/packages/superdough/rng.mjs>
This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU Affero General Public License for more details. You should have received a copy of the GNU Affero General Public License along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

// Imports nothing, so worklets can import it too.
// Without an audio seed every function falls back to Math.random.

// mulberry32: returns floats in [0, 1)
export function mulberry32(seed) {
  let a = seed >>> 0;
  return function () {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// 32-bit FNV-1a over typed parts, returns an unsigned 32-bit integer
export function hashSeed(...parts) {
  const str = parts.map((p) => typeof p + ':' + String(p)).join('\u0000');
  let h = 2166136261;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

let audioSeed;
let lastCycle;
const occurrences = new Map();
const listeners = [];

export function getAudioSeed() {
  return audioSeed;
}

export function setAudioSeed(n) {
  if (n === audioSeed) {
    return;
  }
  audioSeed = n;
  occurrences.clear();
  lastCycle = undefined;
  listeners.forEach((fn) => fn());
}

export function onAudioSeedChange(fn) {
  listeners.push(fn);
}

// a random function for one named use, e.g. makeRandom('noise', 'white', 48000)
export function makeRandom(...keyParts) {
  if (audioSeed === undefined) {
    return Math.random;
  }
  return mulberry32(hashSeed(audioSeed, ...keyParts));
}

export function randomFromSeed(seed) {
  return typeof seed === 'number' ? mulberry32(seed) : Math.random;
}

function sortKeys(v) {
  if (Array.isArray(v)) {
    return v.map(sortKeys);
  }
  if (v !== null && typeof v === 'object') {
    return Object.fromEntries(
      Object.keys(v)
        .sort()
        .map((k) => [k, sortKeys(v[k])]),
    );
  }
  return v;
}

function stable(v) {
  if (v !== null && typeof v === 'object') {
    return JSON.stringify(sortKeys(v));
  }
  return String(v);
}

function valueHash(value) {
  const parts = [];
  Object.keys(value)
    .filter((k) => !k.startsWith('_') && value[k] !== undefined && typeof value[k] !== 'function')
    .sort()
    .forEach((k) => parts.push(k, stable(value[k])));
  return hashSeed(...parts);
}

// a seed for one voice: the same hap at the same cycle gives the same seed,
// identical haps stacked at the same cycle get different ones
export function voiceSeed(onsetCycle, value) {
  if (audioSeed === undefined) {
    return undefined;
  }
  if (onsetCycle !== lastCycle) {
    occurrences.clear();
    lastCycle = onsetCycle;
  }
  const vh = valueHash(value);
  const occurrence = occurrences.get(vh) ?? 0;
  occurrences.set(vh, occurrence + 1);
  return hashSeed(audioSeed, onsetCycle, vh, occurrence);
}
