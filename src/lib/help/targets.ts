import { writable } from 'svelte/store';

interface Target {
  element: HTMLElement;
  text: string;
  description?: HTMLElement;
}

interface Message {
  scope: HTMLDialogElement | null;
  text: string;
}

const targets = new WeakMap<Element, Target>();
const message = writable<Message | null>(null);
export const helpMessage = { subscribe: message.subscribe };

let pendingMessage: Message | null = null;
let awaitingPublish = false;

function setHelp(next: Message | null) {
  pendingMessage = next;
  if (awaitingPublish) return;
  awaitingPublish = true;
  // A focused node can blur while Svelte is removing it. Publish after that update.
  queueMicrotask(() => {
    awaitingPublish = false;
    message.set(pendingMessage);
  });
}

const interactive = [
  'button',
  'input',
  'select',
  'textarea',
  'a[href]',
  'summary',
  '[contenteditable]:not([contenteditable="false"])',
  '[tabindex]',
  '[role^="menuitem"]',
  '[role="option"]',
  '[role="button"]',
].join(',');

let nextId = 0;
let windowHelp: ReturnType<typeof listen> | undefined;

function dialogFor(element: Element | null): HTMLDialogElement | null {
  return element?.closest('dialog') ?? null;
}

function descriptionIds(element: Element, id: string, include: boolean): void {
  const previous = element.getAttribute('aria-describedby');
  const ids = (previous?.trim().split(/\s+/) ?? []).filter((entry) => entry && entry !== id);
  if (include) ids.push(id);
  const next = ids.join(' ') || null;
  if (next === previous) return;
  if (next === null) element.removeAttribute('aria-describedby');
  else element.setAttribute('aria-describedby', next);
}

/** One description stays attached to each control, regardless of the visual help owner. */
export function registerHelpTarget(element: HTMLElement, text: string | undefined) {
  const { ownerDocument } = element;
  const target: Target = { element, text: '' };
  let id: string | undefined;

  function update(next: string | undefined) {
    const text = next?.trim() ?? '';
    if (text === target.text) return;
    target.text = text;
    let description = target.description;
    if (text) {
      if (!description) {
        description = ownerDocument.createElement('span');
        description.id = id ??= `strudeland-help-${++nextId}`;
        description.hidden = true;
        (dialogFor(element) ?? ownerDocument.body).append(description);
        target.description = description;
      }
      description.textContent = text;
      descriptionIds(element, description.id, true);
      targets.set(element, target);
    } else if (description) {
      targets.delete(element);
      descriptionIds(element, description.id, false);
      description.remove();
      target.description = undefined;
    }
    windowHelp?.update(target);
  }

  update(text);
  return { update, destroy: () => update(undefined) };
}

export const helpTarget = registerHelpTarget;

function findTarget(element: Element | null, focus: boolean): Target | undefined {
  if (focus) {
    const id = element?.getAttribute('aria-activedescendant');
    const item = id ? element?.ownerDocument.getElementById(id) : null;
    const target = item ? targets.get(item) : undefined;
    if (target) return target;
  }

  for (let node = element; node; node = node.parentElement) {
    const target = targets.get(node);
    if (target) return target;
    if (node.matches(interactive)) return;
    if (node.localName === 'label') {
      const field = (node as HTMLLabelElement).control;
      if (field) return targets.get(field);
    }
  }
}

function listen(doc: Document) {
  const win = doc.defaultView!;
  let scope: HTMLDialogElement | null = null;
  let hover: Target | undefined;
  let focus: Target | undefined;
  let pointer: { x: number; y: number; element: Element | null } | undefined;
  let hoverWins = false;
  let last: Target | undefined;
  let lastScope: HTMLDialogElement | null = null;
  let lastText: string | undefined;
  let timer: number | undefined;
  let suspended = false;

  function activeDialog(): HTMLDialogElement | null {
    return Array.from(doc.querySelectorAll<HTMLDialogElement>('dialog[open]')).at(-1) ?? null;
  }

  function valid(target: Target | undefined): target is Target {
    return Boolean(
      target?.text &&
      target.element.isConnected &&
      dialogFor(target.element) === scope &&
      !target.element.closest('[hidden], [inert]'),
    );
  }

  function publish() {
    const target =
      (hoverWins && valid(hover) ? hover : undefined) ?? (valid(focus) ? focus : undefined);
    if (target === last && target?.text === lastText && scope === lastScope) return;
    last = target;
    lastText = target?.text;
    lastScope = scope;
    setHelp(target || scope ? { scope, text: target?.text ?? '' } : null);
  }

  function cancelExit() {
    if (timer !== undefined) win.clearTimeout(timer);
    timer = undefined;
  }

  function clearHover() {
    cancelExit();
    hover = undefined;
    hoverWins = false;
    publish();
  }

  function reset(nextScope = activeDialog()) {
    cancelExit();
    scope = nextScope;
    pointer = undefined;
    hover = undefined;
    hoverWins = false;
    focus = suspended ? undefined : findTarget(doc.activeElement, true);
    publish();
  }

  function updateScope(): boolean {
    const nextScope = activeDialog();
    if (nextScope === scope) return false;
    reset(nextScope);
    return true;
  }

  function reconsiderPointer(element: Element | null) {
    if (element?.closest('[data-help-gutter]') && hoverWins && valid(hover)) {
      cancelExit();
      return;
    }

    const target = findTarget(element, false);
    if (valid(target)) {
      if (target !== hover || timer !== undefined) hoverWins = true;
      cancelExit();
      hover = target;
      publish();
    } else if (element?.closest(interactive) || !valid(hover)) {
      clearHover();
    } else if (timer === undefined) {
      // Keep the message briefly while the pointer travels to the Help area.
      timer = win.setTimeout(clearHover, 300);
    }
  }

  function onPointer(event: PointerEvent) {
    if (suspended || event.pointerType === 'touch') return;
    const element = event.target instanceof Element ? event.target : null;
    if (pointer?.element === element) {
      pointer.x = event.clientX;
      pointer.y = event.clientY;
      return;
    }
    pointer = { x: event.clientX, y: event.clientY, element };
    reconsiderPointer(element);
  }

  function trackHit() {
    if (!pointer) return;
    pointer.element = doc.elementFromPoint(pointer.x, pointer.y);
    reconsiderPointer(pointer.element);
  }

  function onScroll() {
    if (hoverWins || !valid(focus)) trackHit();
  }

  function onLeave(event: PointerEvent) {
    if (event.pointerType !== 'touch' && event.relatedTarget === null) {
      pointer = undefined;
      clearHover();
    }
  }

  function onFocus(event: FocusEvent) {
    if (suspended) return;
    updateScope();
    const element = event.type === 'focusin' ? event.target : event.relatedTarget;
    focus = findTarget(element instanceof Element ? element : null, true);
    hoverWins = event.type === 'focusout' && !focus && pointer !== undefined && valid(hover);
    publish();
  }

  function onKeyboard(event: KeyboardEvent) {
    if (
      suspended ||
      ![
        'Tab',
        'ArrowDown',
        'ArrowUp',
        'ArrowLeft',
        'ArrowRight',
        'Home',
        'End',
        'PageUp',
        'PageDown',
      ].includes(event.key)
    )
      return;
    hoverWins = false;
    focus = findTarget(doc.activeElement, true);
    publish();
  }

  function onBlur() {
    suspended = true;
    reset();
  }

  function onWindowFocus() {
    suspended = doc.hidden;
    reset();
  }

  const observer = new MutationObserver((records) => {
    let activeChanged = false;
    let modalChanged = scope !== null && !scope.isConnected;
    for (const record of records) {
      if (record.attributeName === 'aria-describedby') {
        const element = record.target as Element;
        const description = targets.get(element)?.description;
        if (description) descriptionIds(element, description.id, true);
      } else if (
        record.attributeName === 'aria-activedescendant' &&
        record.target === doc.activeElement
      ) {
        activeChanged = true;
      } else if (record.attributeName === 'open' && (record.target as Element).matches('dialog')) {
        modalChanged = true;
      } else if (record.type === 'childList') {
        for (const node of record.addedNodes) {
          if (
            node instanceof Element &&
            (node.matches('dialog[open]') || node.querySelector('dialog[open]'))
          ) {
            modalChanged = true;
            break;
          }
        }
      }
    }
    if ((modalChanged && updateScope()) || suspended) return;
    if (activeChanged || (focus && !valid(focus))) {
      focus = findTarget(doc.activeElement, true);
      hoverWins = false;
    }
    if (hover && !valid(hover)) {
      pointer = undefined;
      clearHover();
    }
    publish();
  });
  observer.observe(doc.body, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ['aria-describedby', 'aria-activedescendant', 'hidden', 'inert', 'open'],
  });

  const stop = new AbortController();
  const capture = { capture: true, signal: stop.signal };
  doc.addEventListener('pointerover', onPointer, capture);
  doc.addEventListener('pointermove', onPointer, capture);
  doc.addEventListener('pointerout', onLeave, capture);
  doc.addEventListener('lostpointercapture', trackHit, capture);
  doc.addEventListener('scroll', onScroll, capture);
  doc.addEventListener('focusin', onFocus, capture);
  doc.addEventListener('focusout', onFocus, capture);
  doc.addEventListener('keydown', onKeyboard, capture);
  doc.addEventListener('visibilitychange', onWindowFocus, capture);
  win.addEventListener('blur', onBlur, { signal: stop.signal });
  win.addEventListener('focus', onWindowFocus, { signal: stop.signal });

  return {
    reset,
    update(target: Target) {
      if (suspended) return;
      const active = doc.activeElement;
      if (
        target.element === active ||
        (target.element.id && active?.getAttribute('aria-activedescendant') === target.element.id)
      ) {
        focus = findTarget(active, true);
      }
      if (target === hover || target === focus || target === last) publish();
    },
    destroy() {
      cancelExit();
      observer.disconnect();
      stop.abort();
      setHelp(null);
    },
  };
}

export function startHelp(doc: Document): () => void {
  windowHelp?.destroy();
  const listener = listen(doc);
  windowHelp = listener;
  listener.reset();
  return () => {
    if (windowHelp !== listener) return;
    listener.destroy();
    windowHelp = undefined;
  };
}

export function resetHelp(): void {
  windowHelp?.reset();
}
