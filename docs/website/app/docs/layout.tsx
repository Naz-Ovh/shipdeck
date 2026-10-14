import { source } from '@/lib/source';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { baseOptions } from '@/lib/layout.shared';
import { SidebarSeparator } from '@/components/sidebar-separator';

export default function Layout({ children }: LayoutProps<'/docs'>) {
  return (
    <DocsLayout
      tree={source.getPageTree()}
      {...baseOptions()}
      sidebar={{ defaultOpenLevel: 1, components: { Separator: SidebarSeparator } }}
    >
      {children}
    </DocsLayout>
  );
}
