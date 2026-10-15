import { cn } from '@/lib/cn';
import { pipelineSteps } from '@/lib/pipeline';

const chip = 'rounded-md bg-sd-3 px-1.5 py-0.5 font-mono text-[0.66rem]';

/** The eleven deploy steps, with the sides that accept your commands. */
export function PipelineSteps({ className }: { className?: string }) {
  return (
    <div className={cn('not-prose', className)}>
      <ol className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-5">
        {pipelineSteps.map((s, i) => (
          <li
            key={s.name}
            className="relative flex flex-col rounded-xl border border-sd-line bg-sd-2 p-3.5 transition-colors hover:border-[color-mix(in_oklab,var(--color-sd-jade)_45%,var(--color-sd-line))]"
          >
            <div className="flex items-center justify-between">
              <span className="font-mono text-[0.7rem] text-sd-ink-faint">{String(i + 1).padStart(2, '0')}</span>
              {s.name === 'switch' && (
                <span className="rounded-md bg-sd-jade-wash px-1.5 py-0.5 font-mono text-[0.62rem] font-semibold uppercase tracking-wider text-sd-jade-strong">
                  goes live
                </span>
              )}
            </div>
            <p className="mt-2 font-mono text-[0.92rem] font-semibold text-sd-ink">{s.name}</p>
            <p className="mt-1 text-[0.84rem] leading-snug text-sd-ink-soft">{s.does}</p>
            <div className="mt-auto flex gap-1.5 pt-3">
              {s.yours.length === 0 ? (
                <span className={cn(chip, 'text-sd-ink-faint')}>shipdeck</span>
              ) : (
                s.yours.map((side) => (
                  <span key={side} className={cn(chip, 'font-medium', side === 'local' ? 'text-sd-moss' : 'text-sd-tide')}>
                    {side}
                  </span>
                ))
              )}
            </div>
          </li>
        ))}
      </ol>
      <p className="mt-3 font-mono text-[0.74rem] text-sd-ink-faint">
        <span className="text-sd-moss">local</span> runs on your PC · <span className="text-sd-tide">server</span> runs
        over SSH · a <span className="text-sd-ink-soft">failed</span> step runs your local commands after any failure
      </p>
    </div>
  );
}
