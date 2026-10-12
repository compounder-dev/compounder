import assert from 'node:assert/strict';
import { test } from 'node:test';
import { axisSteps, byYear, changes, dayText, extent, keyMove, linePath, nearest, startOf, thinned, timeSteps } from './chart.ts';

test('a long line is thinned evenly and keeps both ends', () => {
  assert.deepEqual(thinned(4, 10), [0, 1, 2, 3]);
  assert.deepEqual(thinned(11, 3), [0, 5, 10]);
  const kept = thinned(3619, 800);
  assert.equal(kept.length, 800);
  assert.deepEqual([kept[0], kept.at(-1)], [0, 3618]);
  assert.ok(kept.every((place, index) => index === 0 || place > kept[index - 1]));
});

test('the extent covers every value there is and the value it is told to hold', () => {
  assert.deepEqual(extent([[0.1, null, 0.4], [null, -0.2]]), [-0.2, 0.4]);
  assert.deepEqual(extent([[0.1, 0.4]], 0), [0, 0.4]);
  assert.deepEqual(extent([[null], []]), [0, 0]);
});

test('axis steps are round and stay inside the values', () => {
  assert.deepEqual(axisSteps(0, 1), [0, 0.2, 0.4, 0.6, 0.8, 1]);
  assert.deepEqual(axisSteps(-0.12, 0.31), [-0.1, 0, 0.1, 0.2, 0.3]);
  assert.deepEqual(axisSteps(0, 16.4), [0, 5, 10, 15]);
  assert.deepEqual(axisSteps(0.1, 0.7, 3), [0.2, 0.4, 0.6]);
  assert.deepEqual(axisSteps(2, 2), [2]);
});

test('a gap in the values is a gap in the line', () => {
  assert.equal(linePath([0, 10, 20, 30], [5, null, 7, 8]), 'M0.0 5.0M20.0 7.0L30.0 8.0');
  assert.equal(linePath([0, 10], [null, null]), '');
});

test('the nearest point to a pointer is found at the ends and between', () => {
  const xs = [0, 10, 20, 30];
  assert.deepEqual([-5, 0, 4, 5.1, 14.9, 15.1, 29, 99].map((x) => nearest(xs, x)), [0, 0, 0, 1, 1, 2, 3, 3]);
  assert.equal(nearest([7], 100), 0);
});

test('keys move the crosshair and stop at the ends', () => {
  assert.equal(keyMove('ArrowLeft', 0, 50), 0);
  assert.equal(keyMove('ArrowRight', 3, 50), 4);
  assert.equal(keyMove('PageUp', 3, 50), 8);
  assert.equal(keyMove('PageDown', 3, 50), 0);
  assert.deepEqual([keyMove('Home', 30, 50), keyMove('End', 30, 50)], [0, 49]);
  assert.equal(keyMove('a', 3, 50), null);
});

test('a day is written out for a person', () => {
  assert.equal(dayText('2020-05-12'), '12 May 2020');
  assert.equal(dayText('2020-13-12'), '2020-13-12');
  assert.equal(dayText('soon'), 'soon');
});

test('the time axis names years, or months when there are too few years', () => {
  const days = ['2019-12-30', '2019-12-31', '2020-01-02', '2020-06-01', '2021-01-04', '2021-03-01', '2022-01-03'];
  assert.deepEqual(timeSteps(days, 6), [
    { place: 2, text: '2020' },
    { place: 4, text: '2021' },
    { place: 6, text: '2022' },
  ]);
  // Thinned from the newest back, so the latest year is always named.
  assert.deepEqual(timeSteps(days, 2).map((step) => step.text), ['2020', '2022']);
  assert.deepEqual(timeSteps(['2024-01-30', '2024-01-31', '2024-02-01', '2024-03-01'], 6), [
    { place: 2, text: 'Feb' },
    { place: 3, text: 'Mar' },
  ]);
});

test('a period starts on the first day inside it', () => {
  const days = ['2020-03-02', '2021-03-01', '2021-03-02', '2022-03-01'];
  assert.equal(startOf(days, 1), 1);
  assert.equal(startOf(days, 5), 0);
  assert.equal(startOf([], 1), 0);
});

test('a stretch is measured from its first day, with the comparison set level', () => {
  const both = changes([100, 110, 121], [100, 105, 126], 1);
  assert.deepEqual(both.subject.map((change) => +change.toFixed(6)), [0, 0.1]);
  assert.deepEqual(both.comparison.map((change) => +change.toFixed(6)), [0, 0.2]);
  // A comparison that starts later joins at the subject's level that day.
  const late = changes([100, 150, 300], [null, 50, 75], 0);
  assert.deepEqual(late.comparison, [null, 0.5, 1.25]);
  assert.deepEqual(changes([100, 150], [null, null], 0).comparison, [null, null]);
});

test('each year is measured from the end of the year before', () => {
  const days = ['2020-06-01', '2020-12-31', '2021-06-01', '2021-12-31', '2022-01-03'];
  const rows = byYear(days, [100, 110, 99, 121, 121]);
  assert.deepEqual(rows.map((row) => [row.year, row.change === null ? null : +row.change.toFixed(6)]), [['2022', 0], ['2021', 0.1], ['2020', 0.1]]);
  const late = byYear(days, [null, null, 50, 60, 66]);
  assert.deepEqual(late.map((row) => [row.year, row.change === null ? null : +row.change.toFixed(6)]), [['2022', 0.1], ['2021', 0.2], ['2020', null]]);
});
