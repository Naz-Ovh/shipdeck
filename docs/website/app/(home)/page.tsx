import Link from 'next/link';
import { ArrowRight, Boxes, FileCheck2, KeyRound, Layers, RotateCcw, ServerCog } from 'lucide-react';
import { Terminal } from '@/components/home/terminal';
import { CopyCommand } from '@/components/home/copy-command';
import { PipelineSteps } from '@/components/pipeline-steps';
import { author } from '@/lib/shared';
import { shipdeckVersion } from '@/lib/version';

const install = 'curl -LsSf https://shipdeck.naz.ovh/install.sh | sh';

const yaml = `name: My API (prod)
project: ../../..
ssh:
  host: prod
  user: deploy
service:
  name: my-api
  dir: /opt/my-api
  start: ./my-api
pipeline:
  build:
    run:
      local: cargo zigbuild --release
  pack:
    binary: target/release/my-api
    files:
      include:
        deploy: [.env]
  health:
    url: http://127.0.0.1:8080/health`;

const features = [
  {
    icon: ServerCog,
    title: 'Nothing to install on the server',
    body: 'shipdeck logs in as the user you pick. The first deploy creates the app folder and installs the unit. A missing permission is named at preflight, before anything changes.',
  },
  {
    icon: FileCheck2,
    title: 'Mistakes caught before it starts',
    body: 'Every deploy.yaml is read before the screen opens. Each problem is printed with its file, line and a hint, all at once, so one run shows you everything to fix.',
  },
  {
    icon: Layers,
    title: 'One switch, no half states',
    body: 'Releases unpack next to the live one. Going live only moves the current link, so the paths systemd sees never change. A restart costs about a second or two.',
  },
  {
    icon: RotateCcw,
    title: 'Rollback with a countdown',
    body: 'If the switch, restart or health check fails, shipdeck asks before rolling back. It gives you 30 seconds; no answer means it rolls back.',
  },
  {
    icon: KeyRound,
    title: 'Your SSH, as it is',
    body: 'Hosts can be ~/.ssh/config aliases. Log in with a key, a password, or the alias identity file. New host keys are shown for you to trust; a changed key is refused.',
  },
  {
    icon: Boxes,
    title: 'Any language you build locally',
    body: 'Rust, Go, Node.js, Python: if your PC can build it, shipdeck can ship it. Examples for each live in the repository.',
  },
];

export default function HomePage() {
  const version = shipdeckVersion();

  return (
    <main className="flex flex-1 flex-col">
      {/* hero */}
      <section className="relative overflow-hidden">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 [background-image:linear-gradient(var(--color-sd-line)_1px,transparent_1px),linear-gradient(90deg,var(--color-sd-line)_1px,transparent_1px)] [background-size:44px_44px] opacity-45 [mask-image:radial-gradient(ellipse_70%_60%_at_50%_0%,black,transparent)]"
        />
        <div className="relative mx-auto w-full max-w-6xl px-6 pb-20 pt-16 md:pt-24">
         <div className="grid grid-cols-1 gap-8 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,0.95fr)] lg:items-end lg:gap-14">
          <div className="flex flex-col items-start">
            <p className="sd-eyebrow mb-5 inline-flex items-center gap-2 rounded-full border border-sd-line bg-sd-1 px-3 py-1">
              <span className="size-1.5 rounded-full bg-sd-jade" />
              v{version} · terminal deploys over SSH
            </p>
            <h1 className="sd-display text-[2.7rem] text-sd-ink sm:text-[3.6rem] lg:text-[4.1rem]">
              Build here.
              <br />
              Ship over SSH.
              <br />
              <span className="text-sd-jade">Roll back anything.</span>
            </h1>
          </div>
          <div className="flex flex-col items-start lg:pb-2">
            <p className="max-w-[34rem] text-[1.07rem] leading-relaxed text-sd-ink-soft">
              shipdeck builds your app on your own machine, ships only the files you list to a Linux server, and runs
              it there as a systemd service. Every deploy stays on the server as a release you can switch back to.
            </p>
            <div className="mt-8 flex flex-wrap items-center gap-3">
              <Link
                href="/docs/getting-started/quickstart"
                className="inline-flex items-center gap-2 rounded-xl bg-sd-jade px-5 py-2.5 text-[0.95rem] font-semibold text-fd-primary-foreground transition-colors hover:bg-sd-jade-strong"
              >
                Read the quickstart
                <ArrowRight className="size-4" />
              </Link>
              <Link
                href="/docs"
                className="inline-flex items-center rounded-xl border border-sd-line bg-sd-1 px-5 py-2.5 text-[0.95rem] font-medium text-sd-ink transition-colors hover:bg-sd-2"
              >
                How it works
              </Link>
            </div>
            <div className="mt-6 w-full max-w-[34rem]">
              <CopyCommand command={install} />
            </div>
          </div>
         </div>
          <div className="mt-14 min-w-0">
            <Terminal />
          </div>
        </div>
      </section>

      {/* pipeline */}
      <section className="sd-band sd-band-1">
        <div className="mx-auto w-full max-w-6xl px-6 py-20">
          <div className="max-w-2xl">
            <p className="sd-eyebrow">The pipeline</p>
            <h2 className="sd-heading mt-3 text-3xl text-sd-ink sm:text-[2.2rem]">Eleven steps, and nothing live changes before the switch.</h2>
            <p className="mt-4 text-sd-ink-soft leading-relaxed">
              Every deploy runs the same steps in the same order. You add your own commands where a step can run them,
              and the file says plainly which run on your PC and which on the server.
            </p>
          </div>

          <PipelineSteps className="mt-10" />
        </div>
      </section>

      {/* deploy.yaml + releases */}
      <section className="sd-band">
        <div className="mx-auto grid w-full max-w-6xl grid-cols-1 gap-10 px-6 py-20 lg:grid-cols-2">
          <div className="flex min-w-0 flex-col">
            <p className="sd-eyebrow">One file per deployable</p>
            <h2 className="sd-heading mt-3 text-[1.9rem] text-sd-ink">deploy.yaml says what runs where.</h2>
            <p className="mt-4 text-sd-ink-soft leading-relaxed">
              Each service gets a folder with a deploy.yaml. Point shipdeck at the folder that holds them all and
              every one shows up on the first screen. A JSON Schema gives your editor completion for every field.
            </p>
            <div className="mt-6 overflow-hidden rounded-xl border border-sd-line bg-sd-1">
              <div className="flex items-center gap-2 border-b border-sd-line bg-sd-2 px-4 py-2 font-mono text-[0.74rem] text-sd-ink-faint">
                scripts/deploy/my-api-prod/deploy.yaml
              </div>
              <pre className="overflow-x-auto px-4 py-3.5 font-mono text-[0.78rem] leading-[1.7] text-sd-ink-soft">
                {yaml.split('\n').map((line, i) => {
                  const m = line.match(/^(\s*)([\w.-]+)(:)(.*)$/);
                  return (
                    <span key={i} className="block">
                      {m ? (
                        <>
                          {m[1]}
                          <span className="text-sd-jade-strong">{m[2]}</span>
                          <span className="text-sd-ink-faint">{m[3]}</span>
                          <span className="text-sd-ink">{m[4]}</span>
                        </>
                      ) : (
                        line
                      )}
                    </span>
                  );
                })}
              </pre>
            </div>
          </div>

          <div className="flex min-w-0 flex-col">
            <p className="sd-eyebrow">Releases on the server</p>
            <h2 className="sd-heading mt-3 text-[1.9rem] text-sd-ink">Twenty versions, one link.</h2>
            <p className="mt-4 text-sd-ink-soft leading-relaxed">
              Each deploy unpacks into its own folder and leaves a backup of what it shipped. The newest five stay
              unpacked, the next fifteen as backups, and the Releases screen switches back to any of them.
            </p>
            <div className="mt-6 rounded-xl border border-sd-line bg-sd-1 p-2">
              <div className="rounded-lg bg-sd-2 p-4 font-mono text-[0.78rem] leading-[1.85]">
                <p className="text-sd-ink">/opt/my-api</p>
                <div className="ml-2 border-l border-sd-line-strong pl-4">
                  <p>
                    <span className="text-sd-jade-strong">current</span>
                    <span className="text-sd-ink-faint"> → releases/v0.3.2-77aa001-20261014.160501</span>
                  </p>
                  <p className="text-sd-ink">releases/</p>
                  <div className="ml-2 rounded-md bg-sd-3 py-1 pl-3 pr-2">
                    <p className="flex items-center justify-between gap-3">
                      <span className="truncate text-sd-ink">v0.3.2-77aa001-20261014.160501</span>
                      <span className="rounded bg-sd-jade px-1.5 text-[0.64rem] font-bold text-fd-primary-foreground">LIVE</span>
                    </p>
                    <p className="truncate text-sd-ink-soft">v0.3.1-ab12cd3-20261009.112040</p>
                    <p className="truncate text-sd-ink-soft">v0.3.0-5e6f7a8-20261002.094415</p>
                  </div>
                  <p className="text-sd-ink">backups/</p>
                  <div className="ml-2 rounded-md bg-sd-3 py-1 pl-3 pr-2">
                    <p className="truncate text-sd-ink-faint">v0.3.2-77aa001-20261014.160501.tar.zst</p>
                    <p className="truncate text-sd-ink-faint">v0.3.1-ab12cd3-20261009.112040.tar.zst</p>
                    <p className="truncate text-sd-ink-faint">v0.3.0-5e6f7a8-20261002.094415.tar.zst</p>
                    <p className="truncate text-sd-ink-faint">v0.2.4-1c2d3e4-20260921.171203.tar.zst</p>
                  </div>
                  <p className="text-sd-ink">shared/</p>
                </div>
              </div>
            </div>
            <div className="sd-term mt-4" role="img" aria-label="The rollback question: v0.3.2 failed at Health, roll back to v0.3.1, rolling back in 27 seconds.">
              <div className="sd-term-scroll">
                <div className="sd-term-screen sd-dialog" aria-hidden="true">
                  <span className="sd-dialog-title">my-api-prod · health failed</span>
                  <div className="sd-row">v0.3.2 failed at Health: not healthy after 20s: the health url said: 502</div>
                  <div className="sd-row sd-muted">v0.3.1 was live before it.</div>
                  <div className="sd-row">&nbsp;</div>
                  <div className="sd-row">
                    <b>Roll back to v0.3.1?</b>
                    <span className="sd-push sd-warn">rolling back in 27s</span>
                  </div>
                  <div className="sd-row">&nbsp;</div>
                  <div className="sd-row sd-keys">
                    <span>
                      <kbd>y</kbd> roll back now
                    </span>
                    <span>
                      <kbd>n</kbd> keep v0.3.2 live
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* features */}
      <section className="sd-band sd-band-1">
        <div className="mx-auto w-full max-w-6xl px-6 py-20">
          <p className="sd-eyebrow">Why it is built this way</p>
          <h2 className="sd-heading mt-3 max-w-2xl text-3xl text-sd-ink sm:text-[2.2rem]">
            For servers too small to build on.
          </h2>
          <div className="mt-10 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {features.map((f) => (
              <div key={f.title} className="rounded-xl border border-sd-line bg-sd-2 p-5">
                <div className="mb-4 grid size-9 place-items-center rounded-lg bg-sd-3 text-sd-jade">
                  <f.icon className="size-4.5" />
                </div>
                <h3 className="font-semibold text-sd-ink">{f.title}</h3>
                <p className="mt-2 text-[0.9rem] leading-relaxed text-sd-ink-soft">{f.body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* closing */}
      <section className="sd-band">
        <div className="mx-auto flex w-full max-w-6xl flex-col items-start justify-between gap-6 px-6 py-16 md:flex-row md:items-center">
          <div>
            <h2 className="sd-heading text-2xl text-sd-ink">From zero to the first deploy in one sitting.</h2>
            <p className="mt-2 text-sd-ink-soft">Install, write one file, press Enter.</p>
          </div>
          <div className="flex flex-wrap gap-3">
            <Link
              href="/docs/getting-started/quickstart"
              className="inline-flex items-center gap-2 rounded-xl bg-sd-jade px-5 py-2.5 text-[0.95rem] font-semibold text-fd-primary-foreground transition-colors hover:bg-sd-jade-strong"
            >
              Quickstart <ArrowRight className="size-4" />
            </Link>
            <Link
              href="/docs/reference/deploy-yaml"
              className="inline-flex items-center rounded-xl border border-sd-line bg-sd-1 px-5 py-2.5 text-[0.95rem] font-medium text-sd-ink transition-colors hover:bg-sd-2"
            >
              deploy.yaml reference
            </Link>
          </div>
        </div>
        <footer className="border-t border-sd-line">
          <div className="mx-auto flex w-full max-w-6xl flex-wrap items-center justify-between gap-3 px-6 py-6 font-mono text-[0.74rem] text-sd-ink-faint">
            <span>
              shipdeck {version} · made by{' '}
              <a href={author.url} rel="author" className="underline-offset-4 transition-colors hover:text-sd-ink hover:underline">
                {author.name}
              </a>
            </span>
            <span>Build locally, deploy over SSH as a systemd service, roll back any release.</span>
          </div>
        </footer>
      </section>
    </main>
  );
}
