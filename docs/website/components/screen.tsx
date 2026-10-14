import type { ReactNode } from 'react';

/**
 * A text capture of a shipdeck screen or dialog, framed like a terminal.
 * Pass the capture as a template string: <Screen title="Releases">{`...`}</Screen>
 */
export function Screen({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <figure className="sd-term not-prose my-6">
      <div className="sd-term-bar">
        <span className="sd-term-dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        <span className="sd-term-title">{title ?? 'shipdeck'}</span>
      </div>
      <div className="sd-term-scroll">
        <pre className="sd-screen">{typeof children === 'string' ? children.replace(/^\n+|\n+$/g, '') : children}</pre>
      </div>
    </figure>
  );
}
