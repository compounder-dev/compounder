// Icons are drawn here, inline, on a 20 unit grid in the colour of the text around them.
// Each is one path, so the server can draw the same icon from the same string.

export const ICON_PATHS = {
  'arrow-left': 'M16 10H4m5-5-5 5 5 5',
  'arrow-up': 'M10 16V4m-5 5 5-5 5 5',
  'arrow-down': 'M10 4v12m-5-5 5 5 5-5',
  'chevron-right': 'm8 5 5 5-5 5',
  check: 'm4.5 10.5 3.5 3.5 7.5-8',
  close: 'm5 5 10 10M15 5 5 15',
  clock: 'M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zm0 3.5V10l2.5 2',
  pause: 'M7.5 5.5v9m5-9v9',
  alert: 'M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zm0 3.5v4m0 3h.01',
  info: 'M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zm0 6v4.5m0-7h.01',
  pencil: 'm4 16 .8-3.2L13.6 4a1.4 1.4 0 0 1 2 0l.4.4a1.4 1.4 0 0 1 0 2L7.2 15.2 4 16z',
  home: 'M3.5 9.5 10 4l6.5 5.5V16a.5.5 0 0 1-.5.5h-3.5V12h-5v4.5H4a.5.5 0 0 1-.5-.5V9.5z',
  offline: 'm3 3 14 14M5 9.5a7.4 7.4 0 0 1 2.2-1.4m3.9-.5A7.3 7.3 0 0 1 15 9.5m-7.4 2.9a3.6 3.6 0 0 1 1.7-.8M10 15h.01',
  coin: 'M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zm0 3v8m2-6.2c-.4-.6-1.1-.9-2-.9-1.2 0-2 .6-2 1.4 0 2 4 .9 4 2.8 0 .8-.8 1.4-2 1.4-.9 0-1.7-.4-2.1-1',
} as const;

export type IconName = keyof typeof ICON_PATHS;

export function Icon({ name, size = 'md' }: { name: IconName; size?: 'sm' | 'md' | 'lg' }) {
  return (
    <svg
      className={`icon icon-${size}`}
      viewBox="0 0 20 20"
      width="20"
      height="20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={ICON_PATHS[name]} />
    </svg>
  );
}
