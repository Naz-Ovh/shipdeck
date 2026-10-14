'use client';
import { Check, Copy } from 'lucide-react';
import { useState } from 'react';

export function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="flex items-center gap-2 rounded-xl border border-sd-line bg-sd-2 py-1.5 pl-4 pr-1.5 font-mono text-[0.8rem] text-sd-ink max-w-full">
      <span className="select-none text-sd-jade">$</span>
      <code className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap [scrollbar-width:none]">{command}</code>
      <button
        type="button"
        onClick={copy}
        aria-label={copied ? 'Copied' : 'Copy the command'}
        className="grid size-8 shrink-0 place-items-center rounded-lg bg-sd-3 text-sd-ink-soft transition-colors hover:bg-sd-4 hover:text-sd-ink"
      >
        {copied ? <Check className="size-3.5 text-sd-jade" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  );
}
