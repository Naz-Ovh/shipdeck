import { createGetUrl } from 'fumadocs-core/source';

export const appName = 'shipdeck';
export const appDescription =
  'Build locally, deploy to Linux over SSH with systemd or PM2, and inspect or roll back releases from a terminal UI or CLI.';
export const author = {
  name: 'H Nazmul Hasan Rakib',
  url: 'https://naz.ovh',
  twitter: 'hnazmulhasan',
  github: 'hnazmul',
};
export const docsRoute = '/docs';
export const docsImageRoute = '/og/docs';
export const docsContentRoute = '/llms.mdx/docs';

export const gitConfig = {
  user: 'Naz-Ovh',
  repo: 'shipdeck',
  branch: 'main',
  contentDir: 'docs/website/content/docs',
};

const getContentUrl = createGetUrl(docsContentRoute);

export function getPageMarkdownUrl(page: { slugs: string[]; locale?: string }) {
  const segments = [...page.slugs, 'content.md'];

  return { segments, url: getContentUrl(segments, page.locale) };
}

const getImageUrl = createGetUrl(docsImageRoute);

export function getPageImageUrl(page: { slugs: string[]; locale?: string }) {
  const segments = [...page.slugs, 'image.png'];

  return { segments, url: getImageUrl(segments, page.locale) };
}
