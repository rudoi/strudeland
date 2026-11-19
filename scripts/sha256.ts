import { createHash } from 'node:crypto';

/** Lowercase hex SHA-256 of `data` (a string hashes as UTF-8). */
export const sha256 = (data: Uint8Array | string): string =>
  createHash('sha256').update(data).digest('hex');
