/**
 * Computed colours for canvas code (Strudel's painters, the scope). Colours are named once, in
 * `src/styles/tokens.css`; this reads one back through a hidden probe element, so no colour literal
 * is needed outside that file. Never call it at module load: the stylesheets must be applied.
 */

let probe: HTMLElement | null = null;

function getProbe(): HTMLElement {
  if (!probe || !probe.isConnected) {
    probe = document.createElement('span');
    probe.setAttribute('aria-hidden', 'true');
    probe.style.display = 'none';
    document.documentElement.appendChild(probe);
  }
  return probe;
}

/**
 * The computed colour of a token, given with its dashes (`resolveColor('--scope-trace')`), as
 * `rgb(…)` or `color(srgb …)` (tints made with `color-mix`). Returns `'transparent'` when there is
 * no document or the token does not resolve.
 */
export function resolveColor(token: string): string {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') {
    return 'transparent';
  }
  const el = getProbe();
  // The fallback matters: an unset token makes `var()` invalid at computed-value time, and the
  // probe would then inherit the root's colour instead.
  el.style.color = 'var(' + token + ', transparent)';
  const value = getComputedStyle(el).color.trim();
  return value === '' ? 'transparent' : value;
}

/** Drops the cached probe (tests that swap the global document). */
export function resetResolveProbe(): void {
  probe?.remove();
  probe = null;
}
