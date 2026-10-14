/** The shipdeck mark: a deck of three releases, the top one live. */
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" className={className}>
      <rect x="3" y="14.5" width="18" height="5" rx="1.6" fill="currentColor" opacity="0.28" />
      <rect x="3" y="9.5" width="18" height="5" rx="1.6" fill="currentColor" opacity="0.55" />
      <rect x="3" y="4.5" width="18" height="5" rx="1.6" fill="var(--color-sd-jade)" />
      <circle cx="17.5" cy="7" r="1.1" fill="var(--color-sd-0)" />
    </svg>
  );
}

export function Wordmark() {
  return (
    <span className="inline-flex items-center gap-2">
      <Logo className="size-5.5" />
      <span className="font-semibold tracking-tight text-[0.98rem] [font-variation-settings:'wdth'_96]">
        shipdeck
      </span>
    </span>
  );
}
