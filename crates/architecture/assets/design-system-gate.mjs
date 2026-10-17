#!/usr/bin/env node
// The design system's gate (`design-system` job of .github/workflows/gates.yml):
// the mock-up styles itself only through the design system's tokens and loads
// its stylesheets only from the design system's own files. Run from the
// repository root; no dependency, node alone; it writes nothing.
//
//   node scripts/design-system-gate.mjs
//
// Every file under mock/ is found by glob — mock/design/** and mock/kit/**
// are the design system's own and are never judged. For each file the
// extraction layer yields `{ file, line, property, value, origin }` items
// (origin: css | style-block | inline | js | html): the declarations of a
// `.css` file and of a `<style>` block, inline `style="…"`, `style.X = …` and
// `style.setProperty(…)` in JS, every JS/JSON string literal, and a raw-text
// net over a page's markup. Value checks plug into CHECKS and see every item:
//
//   colours    a hex colour, a colour function or a named colour in a
//              colour-bearing property is refused, `var(--color-x, #abc)`
//              included; only transparent, currentColor, none, inherit,
//              initial and unset pass.
//   tokens     every var(--color-|--font-|--space-|--radius-|--shadow-*,
//              --border-width) must be defined in mock/design/tokens.css.
//   values     a font, a spacing (margin, padding, gap), a border or outline
//              width, an outline offset, a radius or a shadow must be a
//              var(--token), a calc() over tokens, or zero; a font weight only
//              var(--font-weight-heading), normal or inherit. Shapes (width,
//              height, line-height, a percentage radius…), the preludes of
//              @media/@container and JS-computed placement pass — the latter
//              in a file carrying `design-system-gate: computed placement`.
//   loads      no @import, no `<link` in JS, no url() of a stylesheet or a
//              font (a .css or font file, or a remote address that is not an
//              image); data: and local images pass.
//
// Two checks work on the whole file: the three stylesheet links of a page and
// of index.html (in order, one per line, the design system's own files), and
// no `<link`/`@import` in a micro-frontend.
//
// Every violation is an `::error file=…,line=…::` annotation; the verdict is
// given after the last check. apps/ is not scanned: a Micro-UI reaches the
// mock-up through its micro-frontend under mock/components/.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';

const root = process.cwd();
const say = (line) => process.stdout.write(line + '\n');
const posix = (p) => p.split(sep).join('/');

// ---- reporting -------------------------------------------------------------

const findings = new Map();
const escData = (s) => s.replace(/%/g, '%25').replace(/\r/g, '%0D').replace(/\n/g, '%0A');
const escProp = (s) => escData(s).replace(/:/g, '%3A').replace(/,/g, '%2C');
const clip = (s, n = 90) => { const one = s.replace(/\s+/g, ' ').trim(); return one.length > n ? one.slice(0, n - 1) + '…' : one; };

// One annotation per (file, line, offending text): two ways of seeing the same
// literal are one violation.
function flag(file, line, text, message) {
  const key = `${file}\0${line}\0${text}`;
  if (!findings.has(key)) findings.set(key, { file, line, message });
}
const fail = (message) => { findings.set(`::${message}`, { file: null, message }); };

// ---- the files -------------------------------------------------------------

const STYLESHEETS = new Set(['.css']);
const MARKUP = new Set(['.html', '.htm', '.svg']);
const SCRIPTS = new Set(['.js', '.mjs', '.cjs', '.jsx', '.ts', '.mts', '.cts', '.tsx', '.json']);
const extensionOf = (name) => name.slice(name.lastIndexOf('.')).toLowerCase();
const PROTECTED = ['mock/design', 'mock/kit'];

function walk(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir).sort()) {
    if (name === 'node_modules' || name === '.git') continue;
    const path = join(dir, name);
    const rel = posix(relative(root, path));
    if (PROTECTED.includes(rel)) continue;
    if (statSync(path).isDirectory()) walk(path, out);
    else if ([STYLESHEETS, MARKUP, SCRIPTS].some((set) => set.has(extensionOf(name)))) out.push(rel);
  }
  return out;
}

const kindOf = (rel) =>
  /^mock\/pages\/.+\.html$/.test(rel) ? 'page' : /^mock\/components\/.+\.html$/.test(rel) ? 'component' : 'other';

function lineTable(text) {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === '\n') starts.push(i + 1);
  return (offset) => {
    let lo = 0, hi = starts.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (starts[mid] <= offset) lo = mid; else hi = mid - 1; }
    return lo + 1;
  };
}
const newlines = (s) => { let n = 0; for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++; return n; };
const blank = (s) => s.replace(/[^\n]/g, ' ');

// ---- the design system's tokens ---------------------------------------------

function readTokens() {
  const path = join(root, 'mock/design/tokens.css');
  if (!existsSync(path)) return null;
  return new Set([...readFileSync(path, 'utf8').matchAll(/(--[A-Za-z0-9_-]+)\s*:/g)].map((m) => m[1]));
}

// ---- extraction: CSS ---------------------------------------------------------

// Declarations and at-rule statements of a stylesheet, a `<style>` block or an
// inline style attribute. A selector or an at-rule prelude ends in `{` and is
// dropped; a declaration ends in `;` or `}`. Yields `{ property, value, at }`
// where `at` is the offset of the value in `text`.
function* cssDeclarations(text) {
  const clean = text.replace(/\/\*[\s\S]*?(\*\/|$)/g, blank);
  let start = 0, depth = 0, quote = null;
  for (let i = 0; i <= clean.length; i++) {
    const c = i < clean.length ? clean[i] : ';';
    if (quote) { if (c === '\\') i++; else if (c === quote) quote = null; continue; }
    if (c === '"' || c === "'") { quote = c; continue; }
    if (c === '(') depth++;
    else if (c === ')') depth = Math.max(0, depth - 1);
    if (depth) continue;
    if (c === '{') { start = i + 1; continue; }
    if (c !== ';' && c !== '}' && i < clean.length) continue;
    const segment = clean.slice(start, i);
    const lead = segment.length - segment.trimStart().length;
    const body = segment.trim();
    if (body.startsWith('@')) {
      const name = body.match(/^@[\w-]+/)[0].toLowerCase();
      yield { property: name, value: body.slice(name.length), at: start + lead + name.length };
    } else {
      const colon = body.indexOf(':');
      const property = colon > 0 ? body.slice(0, colon).trim().toLowerCase() : '';
      if (/^-{0,2}[a-z_][\w-]*$/i.test(property)) yield { property, value: body.slice(colon + 1), at: start + lead + colon + 1 };
    }
    start = i + 1;
  }
}

// ---- extraction: JS and JSON strings ------------------------------------------

// The string literals of a script (or of a JSON document), with the code that
// precedes each. A template literal's `${…}` is replaced by NULs of the same
// length, so offsets stay exact and a dynamic part is never mistaken for text.
function scanJs(text) {
  const n = text.length;
  const out = [];
  let prev = '', prevWord = '';
  const KEYWORDS = /^(return|typeof|case|in|of|delete|void|throw|new|else|do|yield|await)$/;
  const regexAllowed = () => prev === '' || /[(,=:[!&|?{};+\-*%<>~^]/.test(prev) || (prev === 'a' && KEYWORDS.test(prevWord));
  const push = (start, index, content) => out.push({ index, content, pre: text.slice(Math.max(0, start - 100), start) });

  function string(i, quote) {
    let j = i + 1, content = '';
    while (j < n && text[j] !== quote && text[j] !== '\n') {
      if (text[j] === '\\') { content += text.slice(j, j + 2); j += 2; } else content += text[j++];
    }
    push(i, i + 1, content);
    return j + 1;
  }
  function template(i) {
    let j = i + 1, content = '';
    while (j < n && text[j] !== '`') {
      if (text[j] === '\\') { content += text.slice(j, j + 2); j += 2; }
      else if (text[j] === '$' && text[j + 1] === '{') {
        const [savedPrev, savedWord] = [prev, prevWord];
        prev = ''; prevWord = '';
        const end = code(j + 2, true);
        [prev, prevWord] = [savedPrev, savedWord];
        content += '\0'.repeat(end - j);
        j = end;
      } else content += text[j++];
    }
    push(i, i + 1, content);
    return j + 1;
  }
  function regex(i) {
    let j = i + 1, inClass = false;
    while (j < n && text[j] !== '\n') {
      if (text[j] === '\\') j++;
      else if (text[j] === '[') inClass = true;
      else if (text[j] === ']') inClass = false;
      else if (text[j] === '/' && !inClass) break;
      j++;
    }
    j++;
    while (j < n && /[a-z]/i.test(text[j])) j++;
    return j;
  }
  function code(i, inTemplate) {
    let depth = 0;
    while (i < n) {
      const c = text[i], d = text[i + 1];
      if (c === '/' && d === '/') { while (i < n && text[i] !== '\n') i++; continue; }
      if (c === '/' && d === '*') { const e = text.indexOf('*/', i + 2); i = e < 0 ? n : e + 2; continue; }
      if (c === '"' || c === "'") { i = string(i, c); prev = 'a'; prevWord = ''; continue; }
      if (c === '`') { i = template(i); prev = 'a'; prevWord = ''; continue; }
      if (c === '/') {
        if (regexAllowed()) { i = regex(i); prev = 'a'; prevWord = ''; } else { prev = '/'; i++; }
        continue;
      }
      if (inTemplate) {
        if (c === '{') depth++;
        else if (c === '}') { if (depth === 0) return i + 1; depth--; }
      }
      if (/\s/.test(c)) { i++; continue; }
      if (/[\w$]/.test(c)) {
        let j = i;
        while (j < n && /[\w$]/.test(text[j])) j++;
        prevWord = text.slice(i, j); prev = 'a'; i = j;
        continue;
      }
      prev = c; prevWord = ''; i++;
    }
    return n;
  }
  code(0, false);
  return out;
}

const kebab = (name) => name.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
const SVG_COLOUR_ATTRIBUTES = 'fill|stroke|stop-color|flood-color|lighting-color|color';

// The declarations a piece of markup (a JS string, a page) carries as
// attributes: style="…" and the SVG presentation attributes that take a colour.
function* markupDeclarations(text, at, origin) {
  for (const m of text.matchAll(/(?<![\w-])style\s*=\s*\\?"([^"]*?)\\?"|(?<![\w-])style\s*=\s*\\?'([^']*?)\\?'/g)) {
    const value = m[1] ?? m[2];
    const valueAt = m.index + m[0].indexOf(value, m[0].indexOf('=') + 1);
    for (const d of cssDeclarations(value)) yield { property: d.property, value: d.value, at: at + valueAt + d.at, origin: 'inline' };
  }
  for (const m of text.matchAll(new RegExp(`(?<![\\w-])(${SVG_COLOUR_ATTRIBUTES})\\s*=\\s*\\\\?(?:"([^"\\\\]*)"|'([^'\\\\]*)')`, 'g'))) {
    const value = m[2] ?? m[3];
    yield { property: m[1], value, at: at + m.index + m[0].lastIndexOf(value), origin };
  }
}

// ---- extraction: per file ------------------------------------------------------

function* extract(file, text) {
  const lineOf = lineTable(text);
  const item = (property, value, at, origin) => ({ file, line: lineOf(at), property, value, origin, at });
  const ext = extensionOf(file);

  function* script(source, base) {
    for (const s of scanJs(source)) {
      const at = base + s.index;
      let m;
      // A string anywhere in the rest of the statement is a value: `style.color = ok ? 'red' : 'blue'`.
      if ((m = s.pre.match(/\.style\.([A-Za-z]+)\s*=(?!=)[^;{}]*$/))) yield item(kebab(m[1]), s.content, at, 'js');
      else if ((m = s.pre.match(/\.style\.setProperty\(\s*(['"`])([^'"`]+)\1\s*,[^;{}]*$/))) yield item(m[2], s.content, at, 'js');
      else if (/(?:\.style\.cssText\s*\+?=|setAttribute\(\s*['"]style['"]\s*,)\s*$/.test(s.pre)) {
        for (const d of cssDeclarations(s.content)) yield item(d.property, d.value, at + d.at, 'inline');
      }
      yield { ...item(null, s.content, at, 'js'), pre: s.pre };
      for (const d of markupDeclarations(s.content, at, 'js')) yield item(d.property, d.value, d.at, d.origin);
    }
  }

  if (STYLESHEETS.has(ext)) {
    for (const d of cssDeclarations(text)) yield item(d.property, d.value, d.at, 'css');
  } else if (SCRIPTS.has(ext)) {
    yield* script(text, 0);
  } else {
    // html: <style> and <script> blocks first, then the markup around them.
    const blocks = [];
    let markup = text.replace(/<!--[\s\S]*?(-->|$)/g, blank);
    markup = markup.replace(/<(style|script)\b([^>]*)>([\s\S]*?)<\/\1\s*>/gi, (all, tag, attrs, body, offset) => {
      const bodyAt = offset + 1 + tag.length + attrs.length + 1;
      blocks.push({ tag: tag.toLowerCase(), body, bodyAt });
      return blank(all);
    });
    for (const b of blocks.splice(0)) {
      if (b.tag === 'style') for (const d of cssDeclarations(b.body)) yield item(d.property, d.value, b.bodyAt + d.at, 'style-block');
      else yield* script(b.body, b.bodyAt);
    }
    for (const d of markupDeclarations(markup, 0, 'inline')) yield item(d.property, d.value, d.at, d.origin);
    for (const m of markup.matchAll(/data-props\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) {
      const json = (m[1] ?? m[2]).replace(/&quot;/g, '"').replace(/&#39;|&apos;/g, "'").replace(/&amp;/g, '&');
      yield* script(json, m.index + m[0].indexOf('=') + 2);
    }
    yield { file, line: 1, property: null, value: markup, origin: 'html', at: 0, raw: true };
  }
}
// ---- the value checks ----------------------------------------------------------

const HEX = '(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{4}|[0-9a-f]{3})';
const RE_HEX_VALUE = new RegExp(`(?<![\\w&])#${HEX}(?![\\w-])`, 'gi');
const RE_HEX_ANCHORED = new RegExp(`[:(,]\\s*(#${HEX})(?![\\w-])`, 'gi');
const RE_HEX_ANCHORED_RAW = new RegExp(`[:(,]\\s*["']?(#${HEX})(?![\\w-])`, 'gi');
const RE_HEX_BARE = new RegExp(`^\\s*(#${HEX})\\s*$`, 'i');
const RE_COLOUR_FUNCTION = /(?<![\w-])(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/gi;
const SELECTOR_CALL = /(?:querySelector(?:All)?|closest|matches|getElementById)\(\s*$/;
const COLOUR_PROPERTY = /^(?:--[\w-]+|filter|backdrop-filter|color|fill|stroke(?:-[\w-]+)?|background(?:-[\w-]+)?|border(?:-[\w-]+)?|outline(?:-[\w-]+)?|[\w-]+-shadow|[\w-]+-color|text-decoration|column-rule(?:-[\w-]+)?)$/;
const NAMED_COLOURS = new Set(('aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue ' +
  'chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta ' +
  'darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink ' +
  'deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey ' +
  'honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow ' +
  'lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime ' +
  'limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen ' +
  'mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid ' +
  'palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue ' +
  'saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle ' +
  'tomato turquoise violet wheat white whitesmoke yellow yellowgreen').split(' '));
const DESIGN_TOKEN = /^--(?:(?:color|font|space|radius|shadow)-.*|border-width)$/;

// The offending text in its context: the declaration, or the line of a string or a page.
function snippet(it, at) {
  if (it.property) return clip(`${it.property}: ${it.value}`);
  const from = it.value.lastIndexOf('\n', at) + 1;
  const to = it.value.indexOf('\n', at);
  return clip(it.value.slice(from, to < 0 ? undefined : to));
}

const urlsBlanked = (value) => value.replace(/url\(\s*(?:"[^"]*"|'[^']*'|[^)]*)\)/gi, blank);

function colours(it) {
  const hits = [];
  const add = (at, text) => hits.push({ at, text, message: `colour literal "${text}" in "${snippet(it, at)}": style through a var(--color-*) token of mock/design/tokens.css` });
  const value = urlsBlanked(it.value);
  if (it.property !== null) {
    for (const m of value.matchAll(RE_HEX_VALUE)) add(m.index, m[0]);
    for (const m of value.matchAll(RE_COLOUR_FUNCTION)) add(m.index, m[0]);
    if (COLOUR_PROPERTY.test(it.property)) {
      const words = value
        .replace(/var\(\s*--[\w-]*/gi, blank)
        .replace(/(["'])(?:\\.|(?!\1).)*\1/g, blank);
      for (const m of words.matchAll(/(?<![\w#.-])([a-z]+)(?![\w(-])/gi)) {
        if (NAMED_COLOURS.has(m[1].toLowerCase())) add(m.index, m[1]);
      }
    }
  } else {
    const anchored = it.raw ? RE_HEX_ANCHORED_RAW : RE_HEX_ANCHORED;
    for (const m of value.matchAll(anchored)) add(m.index + m[0].length - m[1].length, m[1]);
    const bare = value.match(RE_HEX_BARE);
    if (bare && !SELECTOR_CALL.test(it.pre ?? '')) add(value.indexOf(bare[1]), bare[1]);
    for (const m of value.matchAll(RE_COLOUR_FUNCTION)) add(m.index, m[0]);
  }
  return hits;
}

function tokens(it, ctx) {
  const hits = [];
  for (const m of it.value.matchAll(/var\(\s*(--[A-Za-z0-9_-]*)/g)) {
    const name = m[1];
    const next = it.value[m.index + m[0].length];
    if (!DESIGN_TOKEN.test(name) || name.endsWith('-') || next === '\0' || ctx.tokens.has(name)) continue;
    hits.push({ at: m.index, text: name, message: `unknown design token ${name}: mock/design/tokens.css does not define it` });
  }
  return hits;
}

const SHEET_OR_FONT = /\.(?:css|woff2?|ttf|otf|eot)(?:[?#].*)?$/i;
const IMAGE = /\.(?:png|jpe?g|gif|svg|webp|avif|ico)(?:[?#].*)?$/i;
const REMOTE = /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i;
function loads(it) {
  const hits = [];
  const add = (at, text, what) => hits.push({ at, text, message: `${what} "${clip(text, 80)}": the mock-up loads no stylesheet or font of its own — the design system's files are loaded by the page, once` });
  if (it.property === '@import') add(0, '@import' + it.value, '@import');
  for (const m of it.value.matchAll(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\)/gi)) {
    const target = (m[1] ?? m[2] ?? m[3]).trim();
    if (/^(?:data|blob):/i.test(target)) continue;
    if (SHEET_OR_FONT.test(target) || (REMOTE.test(target) && !IMAGE.test(target))) add(m.index, m[0], 'url() of a stylesheet or a font');
  }
  if (it.property === null && it.origin === 'js') {
    for (const m of it.value.matchAll(/@import\b/g)) add(m.index, m[0], '@import');
    for (const m of it.value.matchAll(/<link\b/gi)) add(m.index, m[0], '<link');
  }
  return hits;
}

// Rules 13-15: font, spacing, border and outline widths, radii and shadows take
// a `var(--token)`, a `calc()` whose lengths all come from tokens, or zero.
// Shapes have no token and are not in the scope (rule 16): letter-spacing,
// line-height, opacity, z-index, stroke-width, a percentage radius, width,
// height, min/max sizes, aspect-ratio, transform, display.
const SPACING = /^(?:scroll-)?(?:margin|padding)(?:-(?:top|right|bottom|left|block|inline)(?:-(?:start|end))?)?$|^(?:grid-)?(?:(?:row|column)-)?gap$/;
const BORDER = /^(?:border|outline|column-rule)(?:-(?:top|right|bottom|left|block|inline)(?:-(?:start|end))?)?$/;
const BORDER_WIDTH = /^(?:border(?:-(?:top|right|bottom|left|block|inline)(?:-(?:start|end))?)?|outline|column-rule)-width$/;
const RADIUS = /^border(?:-(?:top|bottom|start|end)-(?:left|right|start|end))?-radius$/;
const SHADOW = /^(?:-(?:webkit|moz)-)?(?:box|text)-shadow$/;
const FILTER = /^(?:backdrop-)?filter$/;
// A stylesheet places an element with tokens too; a script's computed offset is placement.
const INSET = /^(?:left|top|right|bottom|inset(?:-(?:block|inline)(?:-(?:start|end))?)?)$/;
// JS-computed placement (a token drags, a panel follows the pointer) is left
// alone in a file that says so: it carries the line PLACEMENT_MARK.
const PLACEMENT_MARK = 'design-system-gate: computed placement';
const placementOf = new Map();
function placesByScript(file) {
  if (!placementOf.has(file)) placementOf.set(file, readFileSync(join(root, file), 'utf8').includes(PLACEMENT_MARK));
  return placementOf.get(file);
}

const LENGTH = /(?<![\w#.-])[+-]?(?:\d+\.?\d*|\.\d+)(?:[a-z]+|%)?(?![\w.])/gi;
const ZERO = /^[+-]?(?:0+\.?0*|\.0+)(?:[a-z]+|%)?$/i;
const FONT_WEIGHT_WORDS = new Set(['normal', 'inherit']);
const FONT_FAMILY_WORDS = new Set(['inherit', 'initial', 'unset', 'revert']);
const BORDER_WIDTH_WORDS = /(?<![\w-])(thin|medium|thick)(?![\w-])/gi;
const FONT_SHORTHAND_WORDS = new Set(['inherit', 'initial', 'unset', 'revert', 'normal', 'italic', 'oblique', 'small-caps', 'caption', 'icon', 'menu', 'message-box', 'small-caption', 'status-bar']);
const TOKEN_ONLY = /^var\(\s*--[\w-]+\s*\)$/;

// `name( … )` calls, balanced: blank their text (same length) when `skip` matches the name.
function blankCalls(value, skip) {
  const call = /(?<![\w-])([a-z-]+)\(/iy;
  let out = '';
  for (let i = 0; i < value.length;) {
    call.lastIndex = i;
    const m = call.exec(value);
    if (m && skip.test(m[1])) {
      let depth = 0, j = i + m[1].length;
      for (; j < value.length; j++) {
        if (value[j] === '(') depth++;
        else if (value[j] === ')' && --depth === 0) { j++; break; }
      }
      out += ' '.repeat(j - i);
      i = j;
    } else out += value[i++];
  }
  return out;
}

// Whether `index` sits inside an open calc(), min(), max() or clamp().
function insideMath(text, index) {
  const open = [];
  for (let i = 0; i < index; i++) {
    if (text[i] === '(') open.push(/(?<![\w-])(?:calc|min|max|clamp)$/i.test(text.slice(0, i)));
    else if (text[i] === ')') open.pop();
  }
  return open.some(Boolean);
}

// The literal lengths of a value: a number that is not zero, with or without a
// unit, outside a `var(--token)` — a fallback after the comma is judged. A
// unitless multiplier inside a `calc()` is not a length. Colours, urls and the
// dynamic parts of a template are not lengths.
function literalLengths(value) {
  const bare = blankCalls(value.replace(/\0/g, ' '), /^(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color|color-mix|light-dark|url)$/i)
    .replace(/var\(\s*--[\w-]+/gi, (m) => m.replace(/[^(]/g, ' '));
  const hits = [];
  for (const m of bare.matchAll(LENGTH)) {
    if (ZERO.test(m[0])) continue;
    if (!/[a-z%]$/i.test(m[0]) && insideMath(bare, m.index)) continue;
    hits.push({ at: m.index, text: m[0] });
  }
  return hits;
}

function values(it) {
  const { property, value, origin, file } = it;
  if (property === null || property.startsWith('--') || property.startsWith('@')) return [];
  const hits = [];
  const add = (at, text, what) => hits.push({ at, text, message: `literal ${what} "${text}" in "${snippet(it, at)}": use a var(--token) of mock/design/tokens.css, a calc() over tokens, or zero` });
  const lengths = (what, keep = () => true) => { for (const l of literalLengths(value)) if (keep(l)) add(l.at, l.text, what); };
  const word = value.replace(/\0/g, ' ').trim().toLowerCase();
  const wordAt = value.search(/\S/);
  if (word === '') return hits;

  if (property === 'font-family') {
    if (!TOKEN_ONLY.test(word) && !FONT_FAMILY_WORDS.has(word)) add(wordAt, value.trim(), 'font family');
  } else if (property === 'font-weight') {
    if (!TOKEN_ONLY.test(word) && !FONT_WEIGHT_WORDS.has(word)) add(wordAt, value.trim(), 'font weight');
  } else if (property === 'font-size') {
    lengths('font size');
    if (/^(?:(?:xx?-)?small|medium|large|x{1,3}-large|smaller|larger)$/.test(word)) add(wordAt, word, 'font size');
  } else if (property === 'font') {
    // The shorthand carries a size, a weight and a family: a size after `/` is a line-height.
    const slash = value.replace(/\/\s*[\d.]+[a-z%]*/gi, (m) => ' '.repeat(m.length));
    for (const l of literalLengths(slash)) add(l.at, l.text, 'font size');
    for (const w of slash.matchAll(/(?<![\w.#-])(bold|bolder|lighter|[1-9]00)(?![\w.-])/gi)) add(w.index, w[1], 'font weight');
    const rest = blankCalls(slash.replace(/\0/g, ' '), /^(?:calc|var)$/i).replace(/(?<![\w.#-])[+-]?[\d.]+[a-z%]*/gi, (m) => ' '.repeat(m.length));
    for (const w of rest.matchAll(/"[^"]*"|'[^']*'|(?<![\w-])[a-z][\w-]*/gi)) {
      if (!FONT_SHORTHAND_WORDS.has(w[0].toLowerCase()) && !/^(?:bold|bolder|lighter)$/i.test(w[0])) add(w.index, w[0], 'font family');
    }
  } else if (SPACING.test(property)) lengths('spacing');
  else if (BORDER_WIDTH.test(property) || BORDER.test(property)) {
    lengths('border width');
    for (const w of value.matchAll(BORDER_WIDTH_WORDS)) add(w.index, w[1], 'border width');
  } else if (property === 'outline-offset') lengths('outline offset');
  else if (RADIUS.test(property)) lengths('radius', (l) => !l.text.endsWith('%'));
  else if (SHADOW.test(property)) {
    if (word !== 'none') lengths('shadow');
  } else if (FILTER.test(property)) {
    for (const m of value.matchAll(/drop-shadow\(/gi)) {
      let depth = 0, end = m.index + m[0].length - 1;
      for (; end < value.length; end++) {
        if (value[end] === '(') depth++;
        else if (value[end] === ')' && --depth === 0) break;
      }
      const from = m.index + m[0].length;
      for (const l of literalLengths(value.slice(from, end))) add(from + l.at, l.text, 'shadow');
    }
  } else if (INSET.test(property) && !((origin === 'js' || origin === 'inline') && placesByScript(file))) {
    lengths('offset', (l) => !l.text.endsWith('%'));
  }
  return hits;
}

const CHECKS = [colours, tokens, loads, values];

// ---- the checks on a whole file ---------------------------------------------------

// A page loads the design system's files, in this order, one `<link>` per
// line. `index.html` sits one level up.
const SHEETS = ['design/tokens.css', 'kit/kit.css', 'style.css'];

function checkLinks(file, text) {
  // index.html sits in mock/, a page one folder down, a nested page one more.
  const up = '../'.repeat(file.split('/').length - 2);
  const expected = SHEETS.map((sheet) => up + sheet);
  const lineOf = lineTable(text);
  const markup = text
    .replace(/<!--[\s\S]*?(-->|$)/g, blank)
    .replace(/<(style|script)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, blank);
  const links = [...markup.matchAll(/<link\b(?:"[^"]*"|'[^']*'|[^>"'])*>/gi)].map((m) => {
    const attr = (name) => m[0].match(new RegExp(`(?<![\\w-])${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, 'i'))?.slice(1).find((v) => v !== undefined);
    return { line: lineOf(m.index), rel: attr('rel'), href: attr('href') };
  });
  const closing = markup.search(/<\/head\s*>/i);
  const where = links.length ? links[links.length - 1].line : closing >= 0 ? lineOf(closing) : 1;
  const sheet = (l) => `<link rel="stylesheet" href="${l}">`;
  const order = expected.map(sheet).join(' then ');

  links.forEach((link, i) => {
    if (i >= expected.length) {
      flag(file, link.line, `<link#${i + 1}`, `a ${i + 1}th <link>: the page loads exactly ${expected.length} stylesheets, ${order}`);
      return;
    }
    if (links.filter((o) => o.line === link.line).length > 1) {
      flag(file, link.line, `<link#${i + 1}`, `link ${i + 1} shares its line with another <link>: one <link rel="stylesheet"> per line`);
    }
    if (link.rel?.toLowerCase() !== 'stylesheet' || link.href !== expected[i]) {
      flag(file, link.line, `<link#${i + 1}`, `link ${i + 1} is rel="${link.rel ?? ''}" href="${link.href ?? ''}", expected ${sheet(expected[i])}: the design system's files are loaded in order, never from a copy`);
    } else if (!existsSync(resolve(root, dirname(file), link.href))) {
      flag(file, link.line, `<link#${i + 1}`, `href="${link.href}" does not resolve to a file`);
    }
  });
  if (links.length < expected.length) {
    flag(file, where, '<link#missing', `${links.length} stylesheet link(s) found, expected ${expected.length}: ${order}`);
  }
}

// A micro-frontend is a fragment: the page it sits in loads the stylesheets.
function checkComponent(file, text) {
  const lineOf = lineTable(text);
  const markup = text.replace(/<!--[\s\S]*?(-->|$)/g, blank);
  for (const m of markup.matchAll(/<link\b/gi)) flag(file, lineOf(m.index), m[0], `<link in a micro-frontend: the page loads the design system's stylesheets, a fragment loads none`);
  for (const m of markup.matchAll(/@import\b/gi)) flag(file, lineOf(m.index), m[0], `@import in a micro-frontend: the page loads the design system's stylesheets, a fragment loads none`);
}

// ---- run --------------------------------------------------------------------------

const files = walk(join(root, 'mock'));
const counts = { page: 0, component: 0, other: 0 };
for (const f of files) counts[kindOf(f)]++;

if (counts.page === 0) fail('no mock/pages/*.html found: the gate has no page to scan');
if (counts.component === 0) fail('no mock/components/*.html found: the gate has no micro-frontend to scan');
if (!files.includes('mock/style.css')) fail('mock/style.css is missing: the gate has no stylesheet to scan');
const tokenSet = readTokens();
if (!tokenSet) fail('mock/design/tokens.css is missing: the design tokens cannot be checked');
if (!files.includes('mock/index.html')) fail('mock/index.html is missing: the gate cannot check its stylesheet links');

for (const file of files) {
  const text = readFileSync(join(root, file), 'utf8');
  const kind = kindOf(file);
  for (const it of extract(file, text)) {
    for (const check of CHECKS) {
      for (const hit of check(it, { tokens: tokenSet ?? new Set() })) {
        flag(it.file, it.line + newlines(it.value.slice(0, hit.at)), hit.text, hit.message);
      }
    }
  }
  if (kind === 'page' || file === 'mock/index.html') checkLinks(file, text);
  if (kind === 'component') checkComponent(file, text);
}

for (const f of [...findings.values()].sort((a, b) => (a.file ?? '').localeCompare(b.file ?? '') || (a.line ?? 0) - (b.line ?? 0))) {
  say(f.file ? `::error file=${escProp(f.file)},line=${f.line}::${escData(f.message)}` : `::error::${escData(f.message)}`);
}
say('::notice::apps/ is not scanned by this gate: a Micro-UI reaches the mock-up through its micro-frontend under mock/components/, checked there');
say(`design-system gate: ${counts.page} pages, ${counts.component} components, ${counts.other} other files scanned under mock/ (mock/design/ and mock/kit/ excluded)${files.includes('mock/style.css') ? ', mock/style.css included' : ''}`);
if (findings.size > 0) {
  say(`design-system gate: ${findings.size} violation(s)`);
  process.exit(1);
}
say('design-system gate: ok');
