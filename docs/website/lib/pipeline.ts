export type Side = 'local' | 'server';

/** The eleven deploy steps, with the sides that accept your commands. */
export const pipelineSteps: { name: string; does: string; yours: Side[] }[] = [
  { name: 'preflight', does: 'checks your PC and the server', yours: ['server', 'local'] },
  { name: 'build', does: 'builds on your PC, names the release', yours: ['local'] },
  { name: 'pack', does: 'packs what the live release lacks of what you list', yours: [] },
  { name: 'upload', does: 'sends one archive over SSH', yours: [] },
  { name: 'unpack', does: 'into releases/<id>, copies and checks the rest', yours: [] },
  { name: 'setup', does: 'your commands while the old release serves', yours: ['server'] },
  { name: 'backup', does: 'packs what a rollback needs into backups/', yours: [] },
  { name: 'switch', does: 'points current at the new release', yours: [] },
  { name: 'restart', does: 'restarts the systemd service', yours: [] },
  { name: 'health', does: 'waits until it is active and answers', yours: ['server', 'local'] },
  { name: 'done', does: 'tidies old releases', yours: ['server', 'local'] },
];
