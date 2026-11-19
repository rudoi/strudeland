import { describe, expect, test } from 'bun:test';
import { noticeRole } from '../src/components/ui/notice';

describe('noticeRole', () => {
  test('danger interrupts', () => {
    expect(noticeRole('danger')).toBe('alert');
  });

  test('warning and neutral are polite', () => {
    expect(noticeRole('warning')).toBe('status');
    expect(noticeRole('neutral')).toBe('status');
  });

  test('an inherited (unset) tone is polite', () => {
    expect(noticeRole(undefined)).toBe('status');
  });
});
