// Release notes are CHANGELOG.md bullets: `- **New|Improved|Fix** text`, under a `## <version>` heading.
// The release workflow publishes the same section as the GitHub release body, so one parser serves both.
import changelog from '../../../CHANGELOG.md?raw';

export const parse = (md) =>
  (md ?? '').split('\n').flatMap((l) => {
    const m = l.match(/^[-*]\s+\*\*(New|Improved|Fix)\*\*:?\s+(.+)$/i);
    return m ? [{ kind: m[1].toLowerCase(), text: m[2].trim() }] : [];
  });

export const forVersion = (v) => parse(changelog.split(/^## /m).find((s) => s.split('\n')[0].trim() === v));

const num = (v) => v.split('.').map((n) => parseInt(n, 10) || 0);
const cmp = (a, b) => { const [x, y] = [num(a), num(b)]; return x[0] - y[0] || x[1] - y[1] || x[2] - y[2]; };

// Every released version up to `current`, newest first, as written in CHANGELOG.md.
export const history = (current) =>
  changelog
    .split(/^## /m)
    .slice(1)
    .map((s) => ({ version: s.split('\n')[0].trim(), items: parse(s) }))
    .filter((r) => r.items.length && cmp(r.version, current) <= 0);
