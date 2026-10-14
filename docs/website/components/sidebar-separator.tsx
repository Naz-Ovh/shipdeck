'use client';
import type * as PageTree from 'fumadocs-core/page-tree';

/** Section label in the docs sidebar: a small mono caption with a rule. */
export function SidebarSeparator({ item }: { item: PageTree.Separator }) {
  return (
    <p data-sd-separator className="mt-7 mb-1.5 flex items-center gap-2 px-2 first:mt-1">
      {item.icon}
      <span>{item.name}</span>
    </p>
  );
}
