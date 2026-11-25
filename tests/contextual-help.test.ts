import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { afterAll, afterEach, beforeEach, expect, test } from 'bun:test';
import { get } from 'svelte/store';
import { helpMessage, registerHelpTarget, resetHelp, startHelp } from '../src/lib/help/targets';

GlobalRegistrator.register();
afterAll(() => GlobalRegistrator.unregister());

let stop: () => void;
beforeEach(() => {
  document.body.replaceChildren();
  stop = startHelp(document);
});
afterEach(() => {
  stop();
  document.body.replaceChildren();
});

function element<T extends HTMLElement>(selector: string): T {
  return document.querySelector<T>(selector)!;
}

function hover(node: Element) {
  node.dispatchEvent(new PointerEvent('pointerover', { bubbles: true, pointerType: 'mouse' }));
}

function keyboard(key: string) {
  document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
}

const rendered = async () => {
  await Promise.resolve();
  return get(helpMessage)?.text || undefined;
};
const mutations = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

test('the latest focus or control entry owns help, and an existing hover yields to keyboard navigation', async () => {
  document.body.innerHTML =
    '<button id="one">one</button><button id="two"><span>two</span></button>';
  const one = element('#one');
  const two = element('#two');
  registerHelpTarget(one, 'First action.');
  registerHelpTarget(two, 'Second action.');

  one.focus();
  expect(await rendered()).toBe('First action.');
  hover(two);
  expect(await rendered()).toBe('Second action.');
  keyboard('Tab');
  hover(element('#two span'));
  expect(await rendered()).toBe('First action.');
  two.focus();
  expect(await rendered()).toBe('Second action.');
  one.focus();
  expect(await rendered()).toBe('First action.');
});

test('disabled controls and native labels can show help while nested controls own their own help', async () => {
  document.body.innerHTML = `
    <button id="previous" disabled>previous</button>
    <label><span>tempo</span><input /></label>
    <div id="container"><button id="nested">nested</button></div>`;
  const tempo = element('input');
  registerHelpTarget(element('#previous'), 'You’re on the first page.');
  registerHelpTarget(tempo, 'Set quarter notes per minute.');
  registerHelpTarget(element('#container'), 'Container help.');

  hover(element('#previous'));
  expect(await rendered()).toBe('You’re on the first page.');
  hover(element('label span'));
  expect(await rendered()).toBe('Set quarter notes per minute.');
  hover(element('#nested'));
  expect(await rendered()).toBeUndefined();
});

test('shows the current unavailable reason when disabling a focused, hovered button blurs it', async () => {
  document.body.innerHTML = '<button>export</button>';
  const button = element<HTMLButtonElement>('button');
  const help = registerHelpTarget(button, 'Write the audio file.');
  hover(button);
  button.focus();
  button.disabled = true;
  help.update('Choose an export folder first.');
  button.blur();
  expect(await rendered()).toBe('Choose an export folder first.');
});

test('keeps a stable description, merges existing IDs, and removes its own reference on cleanup', async () => {
  document.body.innerHTML = '<input aria-label="tempo" aria-describedby="units" />';
  const input = element('input');
  const binding = registerHelpTarget(input, 'Change in quarter notes per minute.');
  const id = input.getAttribute('aria-describedby')!.split(' ')[1]!;
  input.focus();

  expect(await rendered()).toBe('Change in quarter notes per minute.');
  expect(document.getElementById(id)?.textContent).toBe(await rendered());
  input.setAttribute('aria-describedby', 'units invalid');
  binding.update('Enter a positive tempo.');
  await mutations();

  expect(input.getAttribute('aria-label')).toBe('tempo');
  expect(input.getAttribute('aria-describedby')).toBe('units invalid ' + id);
  expect(document.getElementById(id)?.textContent).toBe('Enter a positive tempo.');
  binding.destroy();
  expect(await rendered()).toBeUndefined();
  expect(input.getAttribute('aria-describedby')).toBe('units invalid');
  expect(document.getElementById(id)).toBeNull();
});

test('resolves a composite virtual focus before falling back to the focused control', async () => {
  document.body.innerHTML = '<input><div role="option" id="suggestion">note</div>';
  const input = element('input');
  registerHelpTarget(input, 'Write code.');
  input.focus();
  input.setAttribute('aria-activedescendant', 'suggestion');
  registerHelpTarget(element('#suggestion'), 'Insert this suggestion.');
  await mutations();

  expect(await rendered()).toBe('Insert this suggestion.');
  input.removeAttribute('aria-activedescendant');
  await mutations();
  expect(await rendered()).toBe('Write code.');
});

test('keeps valid help from the active modal and discards removed controls and stale view hover', async () => {
  document.body.innerHTML =
    '<button id="outside">outside</button><dialog><button>inside</button></dialog>';
  const outside = element('#outside');
  const dialog = element<HTMLDialogElement>('dialog');
  const inside = element('dialog button');
  registerHelpTarget(outside, 'Outside help.');
  registerHelpTarget(inside, 'Inside help.');
  hover(outside);
  expect(await rendered()).toBe('Outside help.');
  resetHelp();
  expect(await rendered()).toBeUndefined();

  dialog.setAttribute('open', '');
  inside.focus();
  await rendered();
  expect(get(helpMessage)).toEqual({ scope: dialog, text: 'Inside help.' });
  hover(outside);
  expect(await rendered()).toBe('Inside help.');
  inside.remove();
  await mutations();
  expect(await rendered()).toBeUndefined();
  expect(get(helpMessage)?.scope).toBe(dialog);
  dialog.remove();
  await mutations();
  expect(get(helpMessage)).toBeNull();
});
