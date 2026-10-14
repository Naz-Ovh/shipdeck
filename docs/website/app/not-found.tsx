import Link from 'next/link';
import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { baseOptions } from '@/lib/layout.shared';

export default function NotFound() {
  return (
    <HomeLayout {...baseOptions()}>
      <main className="mx-auto flex w-full max-w-xl flex-1 flex-col items-start justify-center px-6 py-24">
        <p className="sd-eyebrow">404</p>
        <h1 className="sd-heading mt-3 text-4xl text-sd-ink">Nothing deployed here.</h1>
        <p className="mt-4 text-sd-ink-soft leading-relaxed">
          This page does not exist, or it moved. The docs and the search will find what you were after.
        </p>
        <div className="mt-8 flex flex-wrap gap-3">
          <Link
            href="/docs"
            className="inline-flex items-center rounded-xl bg-sd-jade px-5 py-2.5 text-[0.95rem] font-semibold text-fd-primary-foreground transition-colors hover:bg-sd-jade-strong"
          >
            Open the docs
          </Link>
          <Link
            href="/"
            className="inline-flex items-center rounded-xl border border-sd-line bg-sd-1 px-5 py-2.5 text-[0.95rem] font-medium text-sd-ink transition-colors hover:bg-sd-2"
          >
            Home
          </Link>
        </div>
      </main>
    </HomeLayout>
  );
}
