export type Side = 'local' | 'server';

/** The twelve deploy steps, with the sides that accept your commands. */
export const pipelineSteps: { name: string; does: string; yours: Side[] }[] = [
  { name: 'preflight', does: 'checks your PC, the server and the runners', yours: ['server', 'local'] },
  { name: 'build', does: 'names the release, builds, collects and hashes files', yours: ['server', 'local'] },
  { name: 'upload', does: 'packs, sends, unpacks, links shared folders', yours: ['server', 'local'] },
  { name: 'setup', does: 'your commands while the old release serves', yours: ['server', 'local'] },
  { name: 'backup', does: 'packs what a rollback needs into backups/', yours: ['server', 'local'] },
  { name: 'switch', does: 'points current at the new release', yours: ['server', 'local'] },
  { name: 'restart', does: 'restarts every process', yours: ['server', 'local'] },
  { name: 'nginx', does: 'parsed and checked; not built yet', yours: ['server', 'local'] },
  { name: 'health', does: 'runs your checks and fetches the health url', yours: ['server', 'local'] },
  { name: 'done', does: 'tidies old releases, runs your commands', yours: ['server', 'local'] },
  { name: 'rollback', does: 'puts an older release back', yours: ['server', 'local'] },
  { name: 'failed', does: 'your commands after any failure', yours: ['server', 'local'] },
];
