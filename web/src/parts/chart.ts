// The arithmetic of a line chart: which points to draw, where the axis steps
// fall, and which point a pointer is nearest to. Nothing here touches the page.

/** At most `most` places out of `count`, evenly spread, the first and the last always among them. */
export function thinned(count: number, most: number): number[] {
  if (count <= most || most < 2) return Array.from({ length: count }, (_, place) => place);
  return Array.from({ length: most }, (_, step) => Math.round((step * (count - 1)) / (most - 1)));
}

/** The lowest and the highest of every value drawn. `hold` is a value kept in view, such as zero. */
export function extent(lines: (number | null)[][], hold?: number): [number, number] {
  let low = hold ?? Infinity;
  let high = hold ?? -Infinity;
  for (const line of lines) {
    for (const value of line) {
      if (value === null) continue;
      if (value < low) low = value;
      if (value > high) high = value;
    }
  }
  return low <= high ? [low, high] : [0, 0];
}

/** Round steps for an axis from `low` to `high`: about `about` of them, each 1, 2 or 5 times a power of ten apart. */
export function axisSteps(low: number, high: number, about = 4): number[] {
  if (!(high > low)) return [low];
  const rough = (high - low) / about;
  const power = 10 ** Math.floor(Math.log10(rough));
  // The round step nearest to an even split, by how many times larger or smaller it is.
  const apart = (step: number) => Math.abs(Math.log(step / rough));
  const step = [1, 2, 5, 10].map((times) => times * power).reduce((best, each) => (apart(each) < apart(best) ? each : best));
  const first = Math.ceil(low / step - 1e-9);
  const last = Math.floor(high / step + 1e-9);
  // Each step is a whole number of steps from zero, so 0.1 + 0.2 never shows on an axis.
  return Array.from({ length: last - first + 1 }, (_, place) => Number(((first + place) * step).toPrecision(12)));
}

/** A line through the points that have a value, as an SVG path. A gap in the values is a gap in the line. */
export function linePath(xs: number[], ys: (number | null)[]): string {
  let path = '';
  let drawing = false;
  ys.forEach((y, place) => {
    if (y === null) {
      drawing = false;
      return;
    }
    path += `${drawing ? 'L' : 'M'}${xs[place].toFixed(1)} ${y.toFixed(1)}`;
    drawing = true;
  });
  return path;
}

/** The place in the rising list `xs` nearest to `x`. */
export function nearest(xs: number[], x: number): number {
  let low = 0;
  let high = xs.length - 1;
  while (high - low > 1) {
    const middle = (low + high) >> 1;
    if (xs[middle] <= x) low = middle;
    else high = middle;
  }
  return high >= 0 && Math.abs(xs[high] - x) < Math.abs(xs[low] - x) ? high : Math.max(low, 0);
}

/** Where a key moves the crosshair to from `at`, among `count` places. Null for a key that does not move it. */
export function keyMove(key: string, at: number, count: number): number | null {
  const jump = Math.max(1, Math.round(count / 10));
  const moves: Record<string, number> = { ArrowLeft: at - 1, ArrowRight: at + 1, PageDown: at - jump, PageUp: at + jump, Home: 0, End: count - 1 };
  return key in moves ? Math.min(count - 1, Math.max(0, moves[key])) : null;
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** A day written "2020-05-12" as "12 May 2020". Anything else is shown as it stands. */
export function dayText(day: string): string {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  const month = parts && MONTHS[Number(parts[2]) - 1];
  return parts && month ? `${Number(parts[3])} ${month} ${parts[1]}` : day;
}

/**
 * The labels under a chart: the first day of each year, or of each month when the days
 * cover fewer than two years, thinned to at most `most`.
 */
export function timeSteps(days: string[], most: number): { place: number; text: string }[] {
  const firsts = (length: number) => days.flatMap((day, place) => (place > 0 && day.slice(0, length) !== days[place - 1].slice(0, length) ? [place] : []));
  const years = firsts(4);
  const byYear = years.length >= 2;
  const places = byYear ? years : firsts(7);
  const every = Math.max(1, Math.ceil(places.length / Math.max(1, most)));
  return places
    .filter((_, index) => (places.length - 1 - index) % every === 0)
    .map((place) => ({ place, text: byYear ? days[place].slice(0, 4) : (MONTHS[Number(days[place].slice(5, 7)) - 1] ?? days[place]) }));
}

/** The first place whose day is `years` years or less before the last day. Zero when the days do not reach that far back. */
export function startOf(days: string[], years: number): number {
  const last = days[days.length - 1] ?? '';
  const from = `${String(Number(last.slice(0, 4)) - years).padStart(4, '0')}${last.slice(4)}`;
  const place = days.findIndex((day) => day >= from);
  return Math.max(0, place);
}

/**
 * A stretch of a test as the change since its first day: the subject, and the comparison
 * set level with the subject on the first day both have a value.
 */
export function changes(values: number[], beside: (number | null)[], from: number): { subject: number[]; comparison: (number | null)[] } {
  const base = values[from];
  const level = beside.findIndex((value, place) => place >= from && value !== null);
  const scale = level < 0 ? 0 : values[level] / (beside[level] as number);
  return {
    subject: values.slice(from).map((value) => value / base - 1),
    comparison: beside.slice(from).map((value) => (value === null ? null : (value * scale) / base - 1)),
  };
}

/** Each calendar year's change, newest first. The first year starts from the first value, so it may be a part year. */
export function byYear(days: string[], values: (number | null)[]): { year: string; change: number | null }[] {
  const rows: { year: string; change: number | null }[] = [];
  let before: number | null = null;
  let last: number | null = null;
  days.forEach((day, place) => {
    const value = values[place];
    const year = day.slice(0, 4);
    if (rows.length === 0 || rows[rows.length - 1].year !== year) {
      // A year is measured from the last value of the year before it, or from the first value there is.
      before = last;
      rows.push({ year, change: null });
    }
    if (value === null) return;
    if (before === null) before = value;
    rows[rows.length - 1].change = value / before - 1;
    last = value;
  });
  return rows.reverse();
}
