/**
 * Headless Chromium for the render page: a local static server (`dist-render/` and the corpus
 * samples) and one fresh page per render. Imports Playwright: never import this from a test.
 */

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { chromium, type Browser, type Page } from 'playwright';
import type { RenderError } from '../../src/lib/render/contract';
import { HarnessError, pageError, resolveStatic, withTimeout } from './transfer';

export const CALL_TIMEOUT_MS = 60_000;
export const TRANSFER_TIMEOUT_MS = 300_000;
export const POLL_MS = 250;
export const RENDER_TIMEOUT_MS = 1_800_000;
const MAX_MESSAGE = 300;

const ARGS = [
  '--autoplay-policy=no-user-gesture-required',
  '--disable-background-timer-throttling',
  '--disable-renderer-backgrounding',
  '--disable-features=CalculateNativeWinOcclusion,IntensiveWakeUpThrottling',
];

const ROOT = resolve(import.meta.dir, '../..');
const DIST_DIR = resolve(ROOT, 'dist-render');
const SAMPLES_DIR = resolve(ROOT, 'tests/render/corpus/samples');

/** A page with the render page loaded, and the errors its listeners collect. */
export interface RenderPage {
  page: Page;
  /** `pageerror` and console errors, in arrival order. */
  errors: RenderError[];
}

const cut = (s: string): string => s.slice(0, MAX_MESSAGE);
const isMissingBrowser = (e: unknown): boolean =>
  /Executable doesn't exist|playwright install/i.test(e instanceof Error ? e.message : String(e));

export class Chromium {
  private server: ReturnType<typeof Bun.serve> | null = null;
  private browser: Browser | null = null;
  private opening: Promise<void> | null = null;

  /** `http://127.0.0.1:<port>` (a secure context, which `audioWorklet.addModule` needs). */
  get origin(): string {
    if (!this.server) throw new Error('chromium not open');
    return `http://127.0.0.1:${this.server.port}`;
  }

  /** Starts the server and the browser; idempotent. */
  open(): Promise<void> {
    this.opening ??= this.doOpen().catch((e) => {
      this.opening = null;
      throw e;
    });
    return this.opening;
  }

  private async doOpen(): Promise<void> {
    if (!existsSync(resolve(DIST_DIR, 'index.html'))) {
      throw new HarnessError('build-missing', 'render page not built: run bun run render:build');
    }
    this.server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      async fetch(req) {
        const path = new URL(req.url).pathname;
        const file = path.startsWith('/samples/')
          ? resolveStatic(SAMPLES_DIR, path.slice('/samples'.length))
          : resolveStatic(DIST_DIR, path === '/' ? '/index.html' : path);
        if (file === null) return new Response('not found', { status: 404 });
        const f = Bun.file(file);
        if (!(await f.exists())) return new Response('not found', { status: 404 });
        return new Response(f);
      },
    });
    try {
      this.browser = await chromium.launch({
        headless: true,
        args: ARGS,
        timeout: CALL_TIMEOUT_MS,
      });
    } catch (e) {
      this.server.stop(true);
      this.server = null;
      if (isMissingBrowser(e)) {
        throw new HarnessError(
          'chromium-missing',
          'Chromium for Playwright is not installed: run bunx playwright install chromium',
        );
      }
      throw e;
    }
  }

  async version(): Promise<string> {
    await this.open();
    return this.browser!.version();
  }

  /** A fresh page (its own browser context) with the render page ready. */
  async newPage(): Promise<RenderPage> {
    await this.open();
    const page = await this.browser!.newPage();
    const errors: RenderError[] = [];
    page.on('pageerror', (e) => errors.push(pageError(cut(`[page error] ${e.message}`))));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(pageError(`[console] ${cut(m.text())}`));
    });
    try {
      await page.goto(`${this.origin}/`, { waitUntil: 'load', timeout: CALL_TIMEOUT_MS });
      await page.waitForFunction(() => (window as any).strudelandRender?.ready === true, null, {
        timeout: CALL_TIMEOUT_MS,
      });
    } catch (e) {
      await page.close().catch(() => {});
      throw e;
    }
    return { page, errors };
  }

  async close(): Promise<void> {
    const { browser, server } = this;
    this.browser = null;
    this.server = null;
    this.opening = null;
    try {
      if (browser) await withTimeout(browser.close(), CALL_TIMEOUT_MS, 'browser close');
    } finally {
      server?.stop(true);
    }
  }
}
