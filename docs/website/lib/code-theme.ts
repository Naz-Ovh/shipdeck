/**
 * Spruce: the syntax theme for code blocks.
 *
 * Colors are CSS variables (set in app/theme.css for light and dark), so the
 * same theme serves both modes and code always matches the site palette.
 */

const v = (name: string) => `var(--sd-code-${name})`;

const tokenColors = [
  { scope: ['comment', 'punctuation.definition.comment'], settings: { foreground: v('comment'), fontStyle: 'italic' } },
  {
    scope: ['entity.name.tag', 'support.type.property-name', 'keyword.other.definition.ini', 'entity.other.attribute-name'],
    settings: { foreground: v('key') },
  },
  {
    scope: ['string', 'string.quoted', 'string.unquoted', 'string.other.link', 'markup.underline.link'],
    settings: { foreground: v('string') },
  },
  {
    scope: ['constant.numeric', 'constant.language', 'constant.other', 'constant.character', 'support.constant'],
    settings: { foreground: v('constant') },
  },
  {
    scope: ['variable', 'variable.other', 'variable.parameter', 'punctuation.definition.variable', 'meta.interpolation'],
    settings: { foreground: v('variable') },
  },
  {
    scope: ['keyword', 'storage', 'storage.type', 'keyword.control', 'entity.name.section', 'entity.name.type'],
    settings: { foreground: v('keyword') },
  },
  {
    scope: ['entity.name.function', 'support.function', 'entity.name.command', 'support.function.builtin'],
    settings: { foreground: v('function') },
  },
  { scope: ['keyword.operator', 'keyword.operator.pipe', 'keyword.operator.logical'], settings: { foreground: v('operator') } },
  {
    scope: ['punctuation', 'punctuation.separator', 'punctuation.definition', 'meta.brace', 'punctuation.section'],
    settings: { foreground: v('punct') },
  },
  { scope: ['markup.inserted', 'punctuation.definition.inserted'], settings: { foreground: v('inserted') } },
  { scope: ['markup.deleted', 'punctuation.definition.deleted'], settings: { foreground: v('deleted') } },
  { scope: ['markup.bold'], settings: { fontStyle: 'bold' } },
  { scope: ['markup.italic'], settings: { fontStyle: 'italic' } },
];

function spruce(type: 'light' | 'dark') {
  return {
    name: `spruce-${type}`,
    type,
    colors: { 'editor.foreground': v('fg'), 'editor.background': 'transparent' },
    fg: v('fg'),
    bg: 'transparent',
    settings: [{ settings: { foreground: v('fg'), background: 'transparent' } }, ...tokenColors],
    tokenColors,
  };
}

export const codeThemes = { light: spruce('light'), dark: spruce('dark') };
