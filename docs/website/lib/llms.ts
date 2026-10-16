import type * as PageTree from 'fumadocs-core/page-tree';
import { source } from './source';
import { appDescription, appName, getPageMarkdownUrl } from './shared';
import { pipelineSteps } from './pipeline';
import { shipdeckVersion } from './version';

/**
 * llms.txt, llms-full.txt and the per-page Markdown, built from the docs at
 * build time. Pages come in sidebar order, and the site's MDX components are
 * turned into plain Markdown so a model reads the same facts a person sees.
 */

type Page = NonNullable<ReturnType<typeof source.getPage>>;

/** Absolute links when SITE_URL is set (llms.txt asks for full URLs). */
const base = (process.env.SITE_URL ?? '').replace(/\/$/, '');

/** Every page with the sidebar section it sits in, in sidebar order. */
function orderedPages(): { section: string; page: Page }[] {
  const out: { section: string; page: Page }[] = [];
  let section = 'Overview';
  const walk = (nodes: PageTree.Node[]) => {
    for (const node of nodes) {
      if (node.type === 'separator') section = String(node.name ?? section);
      else if (node.type === 'folder') {
        if (node.index) push(node.index);
        walk(node.children);
      } else push(node);
    }
  };
  const push = (item: PageTree.Item) => {
    const page = source.getNodePage(item);
    if (page) out.push({ section, page });
  };
  walk(source.getPageTree().children);
  return out;
}

const calloutLabel: Record<string, string> = {
  info: 'Note',
  warn: 'Warning',
  warning: 'Warning',
  error: 'Caution',
  success: 'Tip',
  idea: 'Tip',
};

function attr(tag: string, name: string): string | undefined {
  return tag.match(new RegExp(`\\b${name}="([^"]*)"`))?.[1] ?? tag.match(new RegExp(`\\b${name}=\\{'([^']*)'\\}`))?.[1];
}

function link(href: string): string {
  return href.startsWith('/') ? base + href : href;
}

function pipelineTable(): string[] {
  return [
    '| # | Step | What shipdeck does | Your commands run on |',
    '| --- | --- | --- | --- |',
    ...pipelineSteps.map(
      (s, i) => `| ${i + 1} | ${s.name} | ${s.does} | ${s.yours.length ? s.yours.join(', ') : 'none (shipdeck only)'} |`,
    ),
    '',
    'A `failed` step runs your local commands after any failure.',
  ];
}

/** Turn the site's MDX (processed, JSX kept) into plain Markdown. */
export function mdxToMarkdown(mdx: string): string {
  const out: string[] = [];
  let quote = 0; // nesting depth of callouts, each a level of `>`
  let fence: { marker: string; indent: number } | null = null;
  let screen: string[] | null = null;
  let tree = -1; // depth inside <Files>, -1 when outside

  const emit = (line: string) => out.push(quote ? `${'> '.repeat(quote)}${line}`.trimEnd() : line);

  for (const raw of mdx.split('\n')) {
    // a screen capture: everything up to `}</Screen> is the text, as is
    if (screen) {
      const end = raw.indexOf('`}</Screen>');
      if (end === -1) {
        screen.push(raw);
        continue;
      }
      screen.push(raw.slice(0, end));
      const text = screen.join('\n').replace(/\\`/g, '`').replace(/\\\$\{/g, '${').replace(/^\n+|\s+$/g, '');
      for (const l of ['```txt', ...text.split('\n'), '```']) emit(l);
      screen = null;
      continue;
    }

    // code fences pass through, dedented to where the fence starts
    const fenceMatch = raw.match(/^(\s*)(`{3,}|~{3,})/);
    if (fence) {
      emit(raw.slice(Math.min(fence.indent, raw.length - raw.trimStart().length)));
      if (fenceMatch && fenceMatch[2].startsWith(fence.marker) && raw.trim() === fenceMatch[2]) fence = null;
      continue;
    }
    if (fenceMatch) {
      fence = { marker: fenceMatch[2], indent: fenceMatch[1].length };
      emit(raw.trimStart());
      continue;
    }

    const line = raw.trim();

    if (/^\{\/\*.*\*\/\}$/.test(line)) continue;

    const screenStart = line.match(/^<Screen\b([^>]*)>\{`(.*)$/);
    if (screenStart) {
      const title = attr(screenStart[1], 'title');
      emit(title ? `Screen capture, ${title}:` : 'Screen capture:');
      emit('');
      screen = [screenStart[2]];
      continue;
    }

    if (/^<PipelineSteps\s*\/>$/.test(line)) {
      for (const l of pipelineTable()) emit(l);
      continue;
    }

    // file trees become an indented text tree
    if (line === '<Files>') {
      emit('```txt');
      tree = 0;
      continue;
    }
    if (tree >= 0) {
      if (line === '</Files>') {
        emit('```');
        tree = -1;
      } else if (line.startsWith('<Folder')) {
        emit(`${'  '.repeat(tree)}${attr(line, 'name')}/`);
        if (!line.endsWith('/>')) tree++;
      } else if (line === '</Folder>') tree--;
      else if (line.startsWith('<File')) emit(`${'  '.repeat(tree)}${attr(line, 'name')}`);
      continue;
    }

    // callouts become block quotes with a label
    const callout = line.match(/^<Callout\b([^>]*)>(.*?)(<\/Callout>)?$/);
    if (callout) {
      const label = calloutLabel[attr(callout[1], 'type') ?? 'info'] ?? 'Note';
      const title = attr(callout[1], 'title');
      quote++;
      emit(`**${title ? `${label}: ${title}` : `${label}:`}**${callout[2] ? ` ${callout[2]}` : ''}`);
      if (callout[3]) quote--;
      continue;
    }
    if (line === '</Callout>') {
      quote = Math.max(0, quote - 1);
      out.push('');
      continue;
    }

    const card = line.match(/^<Card\b(.*?)\/?>(?:<\/Card>)?$/);
    if (card) {
      const title = attr(card[1], 'title') ?? '';
      const href = attr(card[1], 'href');
      const description = attr(card[1], 'description');
      emit(`- ${href ? `[${title}](${link(href)})` : title}${description ? `: ${description}` : ''}`);
      continue;
    }

    const tab = line.match(/^<Tab\b([^>]*)>$/);
    if (tab) {
      emit(`**${attr(tab[1], 'value') ?? ''}**`);
      emit('');
      continue;
    }

    const accordion = line.match(/^<Accordion\b([^>]*)>(.*?)(<\/Accordion>)?$/);
    if (accordion) {
      emit(`#### ${attr(accordion[1], 'title') ?? ''}`);
      emit('');
      if (accordion[2]) emit(accordion[2]);
      continue;
    }

    // wrappers with no content of their own
    if (/^<\/?(Cards|Steps|Step|Tabs|Tab|Accordions|Accordion)\b[^>]*>$/.test(line)) continue;

    // headings carry Fumadocs ids (`## Title [#title]`); drop them
    const text = /^#{1,6} /.test(line) ? line.replace(/\s+\[#[^\]]+\]$/, '') : raw.trimStart();
    emit(text.replace(/\]\((\/[^)\s]*)\)/g, (_m, href: string) => `](${link(href)})`));
  }

  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim();
}

async function pageMarkdown(page: Page): Promise<string> {
  const body = mdxToMarkdown(await page.data.getText('processed'));
  const head = [`# ${page.data.title}`, '', `URL: ${link(page.url)}`];
  if (page.data.description) head.push('', `> ${page.data.description}`);
  return `${head.join('\n')}\n\n${body}\n`;
}

function intro(): string {
  return [
    `# ${appName}`,
    '',
    `> ${appDescription}`,
    '',
    `These are the docs for ${appName} ${shipdeckVersion()}. shipdeck is a terminal UI: you run \`shipdeck <deploy-root>\`, where the deploy root holds one folder per deployable, each with a \`deploy.yaml\`. Deploys run a fixed pipeline (${pipelineSteps.map((s) => s.name).join(', ')}). Each deploy sends only the files the live release lacks, unpacks into its own folder under \`/opt/<app>/releases/\` with the rest copied from the live release and every file checked by SHA-256, and leaves a backup of what it shipped in \`/opt/<app>/backups/\`, and you can roll back to any of them.`,
  ].join('\n');
}

/** llms.txt: a summary and one line per page, linking to its Markdown. */
export async function llmsIndex(): Promise<string> {
  const lines = [
    intro(),
    '',
    `The whole documentation in one file: ${link('/llms-full.txt')}`,
  ];
  let section = '';
  for (const { section: s, page } of orderedPages()) {
    if (s !== section) {
      section = s;
      lines.push('', `## ${section}`, '');
    }
    const description = page.data.description ? `: ${page.data.description}` : '';
    lines.push(`- [${page.data.title}](${link(getPageMarkdownUrl(page).url)})${description}`);
  }
  return `${lines.join('\n')}\n`;
}

/** llms-full.txt: every page, in sidebar order, as plain Markdown. */
export async function llmsFull(): Promise<string> {
  const pages = await Promise.all(orderedPages().map(({ page }) => pageMarkdown(page)));
  return `${intro()}\n\n${pages.join('\n---\n\n')}`;
}

/** One page as Markdown, for /llms.mdx/docs/<slug>/content.md. */
export function llmsPage(page: Page): Promise<string> {
  return pageMarkdown(page);
}
