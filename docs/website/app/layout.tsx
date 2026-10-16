import type { Metadata } from 'next';
import { Google_Sans_Flex, JetBrains_Mono } from 'next/font/google';
import localFont from 'next/font/local';
import { Provider } from '@/components/provider';
import { appDescription, appName, author } from '@/lib/shared';
import './global.css';

const sans = Google_Sans_Flex({
  subsets: ['latin'],
  weight: 'variable',
  axes: ['opsz', 'wdth'],
  variable: '--font-google-sans-flex',
  fallback: ['ui-sans-serif', 'system-ui', 'sans-serif'],
  adjustFontFallback: false,
});

const mono = JetBrains_Mono({
  subsets: ['latin'],
  variable: '--font-jetbrains-mono',
});

// Google's subsets leave out box drawing, arrows and braille, which the
// terminal captures need. These official files fill in only those ranges.
const monoSymbols = localFont({
  src: [
    { path: './fonts/JetBrainsMono-Regular.woff2', weight: '400' },
    { path: './fonts/JetBrainsMono-Bold.woff2', weight: '700' },
  ],
  variable: '--font-jetbrains-mono-symbols',
  declarations: [{ prop: 'unicode-range', value: 'U+2190-21FF, U+2300-23FF, U+2500-25FF, U+2700-27BF, U+2800-28FF' }],
  preload: false,
  adjustFontFallback: false,
});

export const metadata: Metadata = {
  metadataBase: new URL(process.env.SITE_URL ?? 'http://localhost:3000'),
  title: {
    template: `%s · ${appName}`,
    default: `${appName}: build locally, deploy over SSH`,
  },
  description: appDescription,
  authors: [{ name: author.name, url: author.url }],
  creator: author.name,
  twitter: { creator: `@${author.twitter}` },
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable} ${monoSymbols.variable}`} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
