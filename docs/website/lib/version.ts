import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The shipdeck version, read at build time from the workspace Cargo.toml two
 * folders up, so the site never drifts from the release it documents.
 */
export function shipdeckVersion(): string {
  try {
    const toml = readFileSync(join(process.cwd(), '..', '..', 'Cargo.toml'), 'utf8');
    const section = toml.slice(toml.indexOf('[workspace.package]'));
    return section.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? 'unknown';
  } catch {
    return 'unknown';
  }
}
