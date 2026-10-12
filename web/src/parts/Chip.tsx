import { Icon, type IconName } from './Icon.tsx';

const CHIPS = {
  live: { word: 'Live', icon: null },
  waiting: { word: 'Waiting', icon: 'clock' },
  paused: { word: 'Paused', icon: 'pause' },
  'needs-you': { word: 'Needs you', icon: 'alert' },
  draft: { word: 'Draft', icon: 'pencil' },
  'pretend-money': { word: 'Pretend money', icon: 'coin' },
} as const satisfies Record<string, { word: string; icon: IconName | null }>;

export type ChipVariant = keyof typeof CHIPS;

/** The state of a thing in an icon and a word. Colour is spared: only the live dot and "needs you" carry any. */
export function Chip({ variant, children }: { variant: ChipVariant; children?: string }) {
  const { word, icon } = CHIPS[variant];
  return (
    <span className={`chip chip-${variant}`}>
      {icon ? <Icon name={icon} size="sm" /> : <span className="chip-dot" aria-hidden="true" />}
      {children ?? word}
    </span>
  );
}
