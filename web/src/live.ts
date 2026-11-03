import type { LiveFund } from './api.ts';

/** A model id as a person says it: `deepseek/deepseek-v4-pro` is "Deepseek V4 Pro". */
export function modelName(model: string): string {
  const name = model.split('/').pop() ?? model;
  return name
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/** What runs the fund, in a word or two. */
export function kindWord(fund: LiveFund): string {
  if (fund.model) return modelName(fund.model);
  return fund.speed === 'velocity' ? 'Velocity' : 'Compounding';
}

/** Its state in a word. */
export function stateWord(fund: LiveFund): string {
  if (fund.note.startsWith('Paused')) return 'Paused';
  if (fund.holdings.length === 0) return 'Flat';
  return fund.holdings.length === 1 ? '1 position' : `${fund.holdings.length} positions`;
}

const time = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });
const day = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' });

/** A moment as a person reads it: the time today, the day and time before that. */
export function when(t: number, now = Date.now()): string {
  const date = new Date(t);
  const today = new Date(now).toDateString() === date.toDateString();
  return today ? time.format(date) : `${day.format(date)}, ${time.format(date)}`;
}

export function dayOf(t: number): string {
  return day.format(new Date(t));
}
