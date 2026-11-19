<script lang="ts">
  import { getContext } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { SCOPE_CLOCK_KEY, type ScopeClock } from '../../lib/scope/clock';
  import {
    SCOPE_SPECS,
    canvasPixels,
    createScopeRenderer,
    type ScopeRenderer,
    type ScopeSize,
  } from '../../lib/scope/draw';
  import { resolveColor } from '../../lib/theme/resolve';

  type Props = HTMLAttributes<HTMLDivElement> & {
    /** Follow the clock; false keeps the scope at rest. */
    running: boolean;
    size?: ScopeSize;
  };

  let { running, size = 'md', class: className, ...rest }: Props = $props();

  // Outside App there is no clock: the scope draws its rest line once and never subscribes.
  const clock = getContext<ScopeClock | undefined>(SCOPE_CLOCK_KEY);

  let canvas = $state<HTMLCanvasElement>();
  let renderer = $state.raw<ScopeRenderer | null>(null);
  // The live renderer, read (untracked) by the clock effect's cleanup: a renderer replaced by a new
  // size or canvas must not draw again, since its canvas now belongs to the new one.
  let current: ScopeRenderer | null = null;

  // Set up drawing for this canvas and size: colours, sizing, reduced motion, the rest line.
  $effect(() => {
    const el = canvas;
    const spec = SCOPE_SPECS[size];
    if (!el) return;
    const ctx = typeof el.getContext === 'function' ? el.getContext('2d') : null;
    if (!ctx) return; // No 2D context: draw nothing.

    const r = createScopeRenderer({
      ctx,
      spec,
      colours: {
        trace: resolveColor('--scope-trace'),
        glow: resolveColor('--scope-glow'),
        rest: resolveColor('--scope-rest'),
      },
    });

    const fit = (width: number, height: number) => {
      const dpr = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;
      const px = canvasPixels(width, height, dpr);
      el.width = px.width;
      el.height = px.height;
      r.resize(width, height, dpr);
    };
    fit(el.clientWidth || spec.width, el.clientHeight || spec.height);
    r.frame(null);

    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver === 'function') {
      observer = new ResizeObserver((entries) => {
        const box = entries[entries.length - 1]?.contentRect;
        if (box && box.width > 0 && box.height > 0) fit(box.width, box.height);
      });
      observer.observe(el);
    }

    let media: MediaQueryList | null = null;
    const onMotion = (e: MediaQueryListEvent) => r.setReducedMotion(e.matches);
    if (typeof matchMedia === 'function') {
      media = matchMedia('(prefers-reduced-motion: reduce)');
      r.setReducedMotion(media.matches);
      media.addEventListener?.('change', onMotion);
    }

    renderer = current = r;
    return () => {
      observer?.disconnect();
      media?.removeEventListener?.('change', onMotion);
      renderer = current = null;
    };
  });

  // Follow the clock while running; stopping puts the rest line back. When the renderer was
  // replaced, the new one has already drawn its rest line, so the old one draws nothing.
  $effect(() => {
    const r = renderer;
    if (!clock || !r || !running) return;
    const unsubscribe = clock.subscribe((buf) => r.frame(buf));
    return () => {
      unsubscribe();
      if (current === r) r.frame(null);
    };
  });
</script>

<div aria-hidden="true" {...rest} class={['tube', size, className]}>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .tube {
    position: relative;
    flex: none;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px var(--border);
  }
  /* The graticule: a centre line, verticals at a fixed pitch, horizontals at 25% and 75%. */
  .md {
    width: 176px;
    height: var(--control-md);
    border-radius: var(--radius-pill);
    background:
      linear-gradient(var(--scope-grat-major), var(--scope-grat-major)) 0 50% / 100% 1px no-repeat,
      repeating-linear-gradient(90deg, transparent 0 21px, var(--scope-grat) 21px 22px),
      linear-gradient(var(--scope-grat), var(--scope-grat)) 0 25% / 100% 1px no-repeat,
      linear-gradient(var(--scope-grat), var(--scope-grat)) 0 75% / 100% 1px no-repeat,
      var(--scope-tube);
  }
  .sm {
    width: 40px;
    height: 16px;
    border-radius: var(--radius-sm);
    background:
      linear-gradient(var(--scope-grat-major), var(--scope-grat-major)) 0 50% / 100% 1px no-repeat,
      repeating-linear-gradient(90deg, transparent 0 10px, var(--scope-grat) 10px 11px),
      linear-gradient(var(--scope-grat), var(--scope-grat)) 0 25% / 100% 1px no-repeat,
      linear-gradient(var(--scope-grat), var(--scope-grat)) 0 75% / 100% 1px no-repeat,
      var(--scope-tube);
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
