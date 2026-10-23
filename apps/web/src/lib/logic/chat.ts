import type { MessageView } from '../data/types';
export const emoji = {
  wave: '👋',
  sparkles: '✨',
  heart: '💜',
  fire: '🔥',
  eyes: '👀',
  tada: '🎉',
  thumbsup: '👍',
  coffee: '☕',
  rocket: '🚀',
  seedling: '🌱',
  raised_hands: '🙌',
  smile: '😊',
};
export function parseMentions(text: string): string[] {
  return [
    ...new Set(
      [...text.replace(/```[\s\S]*?```|`[^`\n]*`/g, '').matchAll(/(?:^|\s)@([\p{L}\p{N}_-]+)/gu)].map(
        (m) => m[1],
      ),
    ),
  ];
}
export function expandEmoji(text: string): string {
  return text.replace(
    /(```[\s\S]*?```|`[^`\n]*`)|:([a-z_]+):/g,
    (match, code: string | undefined, key: string) => code ?? emoji[key as keyof typeof emoji] ?? match,
  );
}
export function isGrouped(previous: MessageView | undefined, current: MessageView): boolean {
  return (
    !!previous &&
    !previous.deleted &&
    !current.deleted &&
    previous.author.user === current.author.user &&
    current._creationTime >= previous._creationTime &&
    current._creationTime - previous._creationTime < 300_000 &&
    sameDay(previous._creationTime, current._creationTime)
  );
}
export function sameDay(a: number, b: number): boolean {
  return new Date(a).toDateString() === new Date(b).toDateString();
}
export function initials(name: string): string {
  return name
    .split(' ')
    .map((n) => n[0])
    .slice(0, 2)
    .join('');
}
export function formatTime(time: number): string {
  return new Date(time).toLocaleTimeString('en', { hour: 'numeric', minute: '2-digit' });
}
export function formatBytes(n: number): string {
  return n < 1024 * 1024 ? `${Math.ceil(n / 1024)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`;
}
