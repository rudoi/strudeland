import { describe, expect, test } from 'bun:test';
import { pressedAttrs } from '../src/components/ui/button';

describe('pressedAttrs', () => {
  test('unset: no attribute, no class, the tone unchanged', () => {
    expect(pressedAttrs(undefined, undefined)).toEqual({
      ariaPressed: undefined,
      dataTone: undefined,
      pressedClass: false,
    });
    expect(pressedAttrs(undefined, 'danger')).toEqual({
      ariaPressed: undefined,
      dataTone: 'danger',
      pressedClass: false,
    });
  });

  test('true: aria-pressed true, the class and the accent tone', () => {
    expect(pressedAttrs(true, undefined)).toEqual({
      ariaPressed: 'true',
      dataTone: 'accent',
      pressedClass: true,
    });
  });

  test('false: aria-pressed false, no class, the default tone', () => {
    expect(pressedAttrs(false, undefined)).toEqual({
      ariaPressed: 'false',
      dataTone: undefined,
      pressedClass: false,
    });
  });

  test('an explicit tone wins', () => {
    expect(pressedAttrs(true, 'danger').dataTone).toBe('danger');
    expect(pressedAttrs(false, 'warning').dataTone).toBe('warning');
  });
});
