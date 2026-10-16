import { createGetUrl } from 'fumadocs-core/source';

export const appName = 'shipdeck';
export const appDescription =
  'A terminal app that builds your service locally, deploys it over SSH as a systemd service, and rolls back any release.';
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
