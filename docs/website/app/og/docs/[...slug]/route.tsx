import { source } from '@/lib/source';
import { notFound } from 'next/navigation';
import { ImageResponse } from 'next/og';
import { getPageImageUrl } from '@/lib/shared';

export const revalidate = false;

// Social preview: the dark spruce palette, the deck mark and the page title.
const ink = '#e6efec';
const soft = '#9fb3ad';
const bg = '#121b19';
const panel = '#18231f';
const jade = '#5fd3b0';

export async function GET(_req: Request, { params }: RouteContext<'/og/docs/[...slug]'>) {
  const { slug } = await params;
  const page = source.getPage(slug.slice(0, -1));
  if (!page) notFound();

  return new ImageResponse(
    (
      <div
        style={{
          width: '100%',
          height: '100%',
          display: 'flex',
          padding: 40,
          background: bg,
        }}
      >
        <div
          style={{
            flex: 1,
            display: 'flex',
            flexDirection: 'column',
            justifyContent: 'space-between',
            padding: '56px 64px',
            borderRadius: 28,
            border: '1px solid #2a3934',
            background: panel,
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 18 }}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 5 }}>
              <div style={{ width: 54, height: 14, borderRadius: 5, background: jade }} />
              <div style={{ width: 54, height: 14, borderRadius: 5, background: ink, opacity: 0.55 }} />
              <div style={{ width: 54, height: 14, borderRadius: 5, background: ink, opacity: 0.28 }} />
            </div>
            <span style={{ fontSize: 40, fontWeight: 700, color: ink, letterSpacing: -1 }}>shipdeck</span>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
            <span style={{ fontSize: 74, fontWeight: 700, color: ink, letterSpacing: -2.5, lineHeight: 1.05 }}>
              {page.data.title}
            </span>
            {page.data.description && (
              <span style={{ fontSize: 32, color: soft, lineHeight: 1.35 }}>{page.data.description}</span>
            )}
          </div>
          <span style={{ fontSize: 24, color: jade }}>Build locally · deploy over SSH · roll back any release</span>
        </div>
      </div>
    ),
    { width: 1200, height: 630 },
  );
}

export function generateStaticParams() {
  return source.getPages().map((page) => ({
    lang: page.locale,
    slug: getPageImageUrl(page).segments,
  }));
}
