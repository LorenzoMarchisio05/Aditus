// Release notes are CHANGELOG.md bullets: `- **New|Improved|Fix** text`, under a `## <version>` heading.
// The release workflow publishes the same section as the GitHub release body, so one parser serves both.
import changelog from '../../../CHANGELOG.md?raw';

export const parse = (md) =>
  (md ?? '').split('\n').flatMap((l) => {
    const m = l.match(/^[-*]\s+\*\*(New|Improved|Fix)\*\*:?\s+(.+)$/i);
    return m ? [{ kind: m[1].toLowerCase(), text: m[2].trim() }] : [];
  });

export const forVersion = (v) => parse(changelog.split(/^## /m).find((s) => s.split('\n')[0].trim() === v));
