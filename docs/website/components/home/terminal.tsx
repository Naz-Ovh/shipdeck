/**
 * A still of the shipdeck Operation screen in the middle of a deploy,
 * drawn with HTML so it stays sharp and selectable. Strings follow the TUI.
 */

const steps: { n: number; name: string; state: 'done' | 'run' | 'wait'; detail?: string; time?: string }[] = [
  { n: 1, name: 'Preflight', state: 'done', detail: 'x86_64 · glibc 2.39 · 41.2 GiB free', time: '0.6s' },
  { n: 2, name: 'Build', state: 'done', detail: '31.0 MiB · glibc ≥ 2.34', time: '1m04s' },
  { n: 3, name: 'Pack', state: 'done', detail: '5 files · 31.2 MiB → 11.8 MiB', time: '1.9s' },
  { n: 4, name: 'Upload', state: 'run', detail: '59%', time: '1.7s' },
  { n: 5, name: 'Unpack', state: 'wait' },
  { n: 6, name: 'Setup', state: 'wait' },
  { n: 7, name: 'Backup', state: 'wait' },
  { n: 8, name: 'Switch', state: 'wait' },
  { n: 9, name: 'Restart', state: 'wait' },
  { n: 10, name: 'Health', state: 'wait' },
  { n: 11, name: 'Done', state: 'wait' },
];

type LogKind = 'step' | 'cmd' | 'info' | 'out' | 'ok' | 'warn';

const log: [string, LogKind, string][] = [
  ['00:00', 'step', 'Preflight: checking your PC and the server'],
  ['00:00', 'cmd', 'connect deploy@prod → 203.0.113.10:22'],
  ['00:00', 'info', 'server: x86_64 · glibc 2.39 · 41.2 GiB free'],
  ['00:00', 'step', 'Build: building on your PC'],
  ['00:00', 'cmd', 'cargo zigbuild --release'],
  ['00:02', 'out', '   Compiling my-api v0.3.2'],
  ['01:05', 'out', '    Finished `release` profile [optimized]'],
  ['01:05', 'step', 'Pack: packing the release'],
  ['01:07', 'ok', 'deploy folder: .env (1.2 KiB)'],
  ['01:07', 'step', 'Upload: sending the release'],
  ['01:07', 'cmd', 'deploy@prod › upload 11.8 MiB'],
];

const marker: Record<LogKind, string> = {
  step: '━━',
  cmd: '›',
  info: 'ℹ',
  out: ' ',
  ok: '✔',
  warn: '▲',
};

export function Terminal() {
  return (
    <div className="sd-term" role="img" aria-label="The shipdeck terminal app halfway through a deploy of my-api: three of eleven steps done, upload at 59 percent.">
      <div className="sd-term-bar">
        <span className="sd-term-dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        <span className="sd-term-title">shipdeck ./scripts/deploy</span>
      </div>

      <div className="sd-term-scroll">
        <div className="sd-term-screen" aria-hidden="true">
          <div className="sd-row">
            <span className="sd-badge">◆ shipdeck</span>
            <span className="sd-muted">root</span> ./scripts/deploy
            <span className="sd-push sd-muted">login</span> key ~/.ssh/id_ed25519
          </div>
          <div className="sd-row sd-tabs">
            <span className="sd-tab">1 Deployables</span>
            <span className="sd-tab sd-tab-on">2 Operation</span>
            <span className="sd-tab">3 Releases</span>
            <span className="sd-push sd-accent">
              <span className="sd-spin" /> Deploying my-api-prod · Upload 59%
            </span>
          </div>

          <div className="sd-panel sd-banner">
            <span className="sd-panel-title">Deploy</span>
            <span className="sd-panel-title sd-panel-title-end">1m09s</span>
            <div className="sd-row">
              <b>My API (prod)</b>
              <span className="sd-muted">→</span> deploy@prod:/opt/my-api
              <span className="sd-muted">release</span>
              <span className="sd-violet">v0.3.2-77aa001-20261014.160501</span>
            </div>
            <div className="sd-row">
              <span className="sd-accent">
                <span className="sd-spin" /> Upload…
              </span>
              <span className="sd-muted">step 4 of 11</span>
            </div>
          </div>

          <div className="sd-split">
            <div className="sd-col">
              <div className="sd-panel">
                <span className="sd-panel-title">Steps</span>
                {steps.map((s) => (
                  <div key={s.n} className="sd-row sd-step" data-state={s.state}>
                    <span className="sd-step-icon">{s.state === 'done' ? '✔' : s.state === 'run' ? <span className="sd-spin" /> : '○'}</span>
                    <span className="sd-step-name">
                      {s.n} {s.name}
                    </span>
                    <span className="sd-step-detail">{s.detail}</span>
                    <span className="sd-step-time">{s.time}</span>
                  </div>
                ))}
              </div>
              <div className="sd-panel sd-progress">
                <span className="sd-panel-title">Progress</span>
                <div className="sd-row sd-accent">
                  <span className="sd-spin" /> Upload <span className="sd-muted">1.7s</span>
                </div>
                <div className="sd-gauge">
                  <span style={{ width: '59%' }} />
                  <em>59%</em>
                </div>
                <div className="sd-row">7.0 MiB of 11.8 MiB</div>
                <div className="sd-row sd-muted">4.2 MiB/s</div>
                <div className="sd-row sd-overall">
                  <span className="sd-muted">overall</span>
                  <span className="sd-line">
                    <span style={{ width: '30%' }} />
                  </span>
                </div>
                <div className="sd-row sd-muted">3/11 steps · 1m09s</div>
              </div>
            </div>

            <div className="sd-panel sd-log">
              <span className="sd-panel-title">Log · {log.length} lines</span>
              <span className="sd-panel-title sd-panel-title-end sd-ok">following</span>
              <div className="sd-log-lines">
                {log.map(([t, kind, text], i) => (
                  <div key={i} className="sd-row sd-logline" data-kind={kind}>
                    <span className="sd-faint">{t}</span>
                    <span className="sd-mark">{marker[kind]}</span>
                    <span className="sd-logtext">{text}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div className="sd-row sd-keys">
            {[
              ['↑↓', 'scroll'],
              ['pgup/pgdn', 'page'],
              ['G', 'follow'],
              ['r', 'releases'],
              ['esc', 'back'],
              ['q', 'quit'],
            ].map(([k, label]) => (
              <span key={k}>
                <kbd>{k}</kbd> {label}
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
