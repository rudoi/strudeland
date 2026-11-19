import { describe, expect, test } from 'bun:test';
import { parseArgs, pickIdentity } from '../scripts/app-signed';

const LISTING = `  1) 0A87729F553C81BA219F7EBA6F0EB3027C9E010E "Apple Development: Jo Example (ABCDE12345)"
  2) 4860E41971D8440C9E5FED633E0F2646FABFFFD9 "Apple Distribution: Jo Example (TEAM123456)"
  3) 4F477FB580D35DEC6F9D04F13CAB0A2096FCB9A8 "Developer ID Application: Jo Example (TEAM123456)"
     3 valid identities found
`;

describe('pickIdentity', () => {
  test('the environment wins', () => {
    expect(pickIdentity(LISTING, 'Developer ID Application: Jo Example (TEAM123456)')).toEqual({
      identity: 'Developer ID Application: Jo Example (TEAM123456)',
    });
  });

  test('a blank environment value is ignored', () => {
    expect(pickIdentity(LISTING, '  ')).toEqual({
      identity: 'Apple Development: Jo Example (ABCDE12345)',
    });
  });

  test('the first Apple Development identity is chosen', () => {
    const reordered = LISTING.split('\n').slice(1, 3).concat(LISTING.split('\n')[0]).join('\n');
    expect(pickIdentity(reordered, undefined)).toEqual({
      identity: 'Apple Development: Jo Example (ABCDE12345)',
    });
  });

  test('no Apple Development identity is an error', () => {
    const r = pickIdentity('     0 valid identities found\n', undefined);
    expect('error' in r && r.error).toContain('Apple Development');
  });
});

describe('parseArgs', () => {
  test('opens by default', () => {
    expect(parseArgs([])).toEqual({ open: true });
  });

  test('--no-open only builds', () => {
    expect(parseArgs(['--no-open'])).toEqual({ open: false });
  });

  test('anything else is an error', () => {
    const r = parseArgs(['--release']);
    expect('error' in r && r.error).toContain('unknown argument --release');
  });
});
