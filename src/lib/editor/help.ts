import { ViewPlugin, type EditorView } from '@codemirror/view';
import { registerHelpTarget } from '../help/targets';

const controls = '.cm-slider input[type="range"], .cm-tooltip-autocomplete [role="option"]';

export const editorHelp = ViewPlugin.fromClass(
  class {
    private readonly registered = new Map<HTMLElement, () => void>();
    private readonly observer: MutationObserver;
    private readonly content: ReturnType<typeof registerHelpTarget>;

    constructor(private readonly view: EditorView) {
      const modifier = /Mac|iP(?:hone|ad|od)/.test(navigator.platform) ? 'Command' : 'Ctrl';
      this.content = registerHelpTarget(
        view.contentDOM,
        `Write Strudel code. Press ${modifier}+Enter to evaluate and play it.`,
      );
      this.observer = new MutationObserver(() => this.sync());
      this.observer.observe(view.dom, { subtree: true, childList: true });
      this.sync();
    }

    private sync() {
      for (const [element, dispose] of this.registered) {
        if (this.view.dom.contains(element)) continue;
        dispose();
        this.registered.delete(element);
      }

      for (const element of this.view.dom.querySelectorAll<HTMLElement>(controls)) {
        if (this.registered.has(element)) continue;
        if (element instanceof HTMLInputElement) {
          const wrapper = element.parentElement!;
          // The vendored widget hides the wrapper, including the interactive range input.
          wrapper.removeAttribute('aria-hidden');
          element.setAttribute('aria-label', 'Pattern slider');
          const { destroy } = registerHelpTarget(
            element,
            'Change the value in your code and the evaluated pattern.',
          );
          this.registered.set(element, () => {
            destroy();
            element.removeAttribute('aria-label');
            wrapper.setAttribute('aria-hidden', 'true');
          });
        } else {
          const { destroy } = registerHelpTarget(element, 'Insert this suggestion into the code.');
          this.registered.set(element, destroy);
        }
      }
    }

    destroy() {
      this.observer.disconnect();
      this.content.destroy();
      for (const dispose of this.registered.values()) dispose();
      this.registered.clear();
    }
  },
);
