/**
 * The `String(number)` differential's Chromium side (hand-run; imports Playwright, so it is not a
 * test): generates the fixed set `S` (`string-number-lib.ts`), writes its hex inputs to
 * `target/string-number-inputs.json`, evaluates `String(x)` and `JSON.stringify([x])` for each
 * in a Chromium page and writes `target/string-number-chromium.json`. Then run
 * `cargo test --release -p strudeland-host --test string_number -- --ignored --nocapture`.
 *
 *   bun scripts/host/string-number.probe.ts
 *
 * Exit 0 written, 1 Chromium failed (`bunx playwright install chromium` once).
 */

import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { COUNTS, generateInputs, SEED } from './string-number-lib';

const ROOT = resolve(import.meta.dir, '../..');
const OUT = resolve(ROOT, 'target');

const { hex } = generateInputs();
mkdirSync(OUT, { recursive: true });
writeFileSync(
  resolve(OUT, 'string-number-inputs.json'),
  `${JSON.stringify({ seed: SEED, counts: COUNTS, hex })}\n`,
);

let browser;
try {
  browser = await chromium.launch({ headless: true });
} catch (e) {
  console.error(`string-number: cannot launch Chromium: ${(e as Error).message}`);
  console.error('install it once with: bunx playwright install chromium');
  process.exit(1);
}
try {
  const page = await browser.newPage();
  const version = browser.version();
  const out = await page.evaluate((inputs: string[]) => {
    const view = new DataView(new ArrayBuffer(8));
    const string: string[] = [];
    const json: string[] = [];
    for (const h of inputs) {
      view.setBigUint64(0, BigInt(`0x${h}`));
      const x = view.getFloat64(0);
      string.push(String(x));
      json.push(JSON.stringify([x]));
    }
    return { string, json };
  }, hex);
  writeFileSync(
    resolve(OUT, 'string-number-chromium.json'),
    `${JSON.stringify({ chromium: version, string: out.string, json: out.json })}\n`,
  );
  console.log(
    `string-number: ${hex.length} inputs, Chromium ${version}: wrote target/string-number-inputs.json and target/string-number-chromium.json`,
  );
} finally {
  await browser.close();
}
