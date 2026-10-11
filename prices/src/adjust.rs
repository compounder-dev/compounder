//! Raw daily bars, and the adjusted series a backtest runs on.
//!
//! A bar is what happened on one day: the close the tape printed, and the
//! dividend and split that took effect that day. A day that is over never
//! changes, so bars are what is stored. The adjusted series is worked out
//! from them each time it is wanted, because it does change: every new
//! dividend or split moves every earlier price.
//!
//! The adjusted price of a day is its close, divided by every later split,
//! times (1 - dividend / previous close) for every later dividend. A split
//! then leaves no jump. A dividend counts as reinvested on its ex-date, at no
//! cost: the change on that day is close / (previous close - dividend) - 1.
//! A special dividend goes the same way. Nothing comes after the last day, so
//! its price is the real close and the whole series is tied to that day.
//!
//! Prices are `f64`, rounded to six decimals. A price series is read for
//! ratios and averages, which is float work, and six decimals is the
//! precision of the series we check ours against. Those are rounded, not cut
//! off: rounding matches them to the last digit and cutting misses on half
//! the days. A whole number of millionths would hold the same values, but the
//! early prices of a fund that has been reverse split many times run to
//! hundreds of billions of dollars, close to where a 64-bit count overflows,
//! and every reader would turn it back into a float anyway. Money that changes
//! hands is not kept here. The engine holds that in whole units.

use crate::day::Day;

/// What one ticker did on one day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub day: Day,
    /// The close as printed that day, in dollars. Not adjusted for anything.
    pub close: f64,
    /// Cash per share for a dividend whose ex-date is this day, counted in
    /// this day's shares. Zero when there is none.
    pub dividend: f64,
    /// New shares for each old share when a split takes effect this day: 2.0
    /// for a 2-for-1, 0.2 for a 1-for-5. One when there is none.
    pub split: f64,
}

impl Bar {
    /// A day with a close and nothing else.
    pub fn new(day: Day, close: f64) -> Bar {
        Bar { day, close, dividend: 0.0, split: 1.0 }
    }
}

/// The adjusted closes of one ticker, oldest day first, on the days it has a bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    days: Vec<Day>,
    prices: Vec<f64>,
}

impl Series {
    /// Never zero: a series is made from at least one bar.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn days(&self) -> &[Day] {
        &self.days
    }

    /// One price per day, in step with `days`.
    pub fn prices(&self) -> &[f64] {
        &self.prices
    }

    pub fn first_day(&self) -> Day {
        self.days[0]
    }

    pub fn last_day(&self) -> Day {
        self.days[self.days.len() - 1]
    }

    /// The day the series is tied to: its price there is the real close, and
    /// every earlier price is adjusted as of that day.
    pub fn as_of(&self) -> Day {
        self.last_day()
    }

    /// Where `day` sits in the series, when the ticker has a bar that day.
    pub fn position(&self, day: Day) -> Option<usize> {
        self.days.binary_search(&day).ok()
    }

    /// The price on `day`, when the ticker has a bar that day.
    pub fn on(&self, day: Day) -> Option<f64> {
        self.position(day).map(|index| self.prices[index])
    }
}

/// Six decimals, the precision adjusted prices are kept to.
const PRICE_SCALE: f64 = 1e6;

fn round_price(value: f64) -> f64 {
    (value * PRICE_SCALE).round() / PRICE_SCALE
}

/// Why `bars` cannot be a ticker's history, if they cannot: they must be in
/// day order with no day twice, every close must be a price, and no dividend
/// may be as large as the close before it.
pub fn validate(bars: &[Bar]) -> Result<(), String> {
    if bars.is_empty() {
        return Err("there are no days".into());
    }
    for (index, bar) in bars.iter().enumerate() {
        let day = bar.day;
        if !bar.close.is_finite() || bar.close <= 0.0 {
            return Err(format!("the close on {day} is {}, which is not a price", bar.close));
        }
        if !bar.dividend.is_finite() || bar.dividend < 0.0 {
            return Err(format!("the dividend on {day} is {}, which is not an amount", bar.dividend));
        }
        if !bar.split.is_finite() || bar.split <= 0.0 {
            return Err(format!("the split on {day} is {}, which is not a ratio", bar.split));
        }
        let Some(before) = index.checked_sub(1).map(|previous| &bars[previous]) else { continue };
        if before.day >= day {
            return Err(format!("{day} comes after {}, so the days are out of order", before.day));
        }
        if bar.dividend >= before.close / bar.split {
            return Err(format!("the dividend of {} on {day} is as large as the close before it", bar.dividend));
        }
    }
    Ok(())
}

/// The adjusted series of one ticker's bars.
pub fn adjust(bars: &[Bar]) -> Result<Series, String> {
    validate(bars)?;
    let mut prices = vec![0.0; bars.len()];
    // Walk back from the last day. `splits` is how many of today's shares one
    // share of the day in hand has become. `factor` is what the dividends
    // paid since then take off its price.
    let (mut splits, mut factor) = (1.0, 1.0);
    for index in (0..bars.len()).rev() {
        let bar = &bars[index];
        prices[index] = round_price(bar.close / splits * factor);
        splits *= bar.split;
        // A dividend on the first day has no earlier price to adjust.
        if bar.dividend > 0.0 && index > 0 {
            // The dividend is in this day's shares, so count the close before it in them too.
            factor *= 1.0 - bar.dividend / (bars[index - 1].close / bar.split);
        }
    }
    Ok(Series { days: bars.iter().map(|bar| bar.day).collect(), prices })
}

/// Fixtures shared by this crate's tests. Each is real history, cut down: the
/// raw bars a public feed gives, and beside them the price that the series
/// we check ours against holds for the same day. That price is the answer to
/// reproduce.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// A large dividend payer, 2012 to 2026: 59 dividends and one 2-for-1 split.
    pub const DIVIDEND_PAYER: &str = include_str!("../fixtures/dividend_payer.csv");
    /// A stock that pays nothing, over the same years: a 7-for-1 and a 10-for-1 split.
    pub const SPLIT_STOCK: &str = include_str!("../fixtures/split_stock.csv");
    /// A long bond fund with a payout every month, over the same years.
    pub const BOND_FUND: &str = include_str!("../fixtures/bond_fund.csv");
    /// The broadest index fund, over the same years. Its days are the calendar.
    pub const INDEX_FUND: &str = include_str!("../fixtures/index_fund.csv");
    /// A 2025 listing, from its first real trading day.
    pub const NEW_LISTING: &str = include_str!("../fixtures/new_listing.csv");
    /// A small leveraged fund in 2015, when it went days at a time without a trade. Bars only.
    pub const THIN_FUND: &str = include_str!("../fixtures/thin_fund.csv");

    /// The bars of a fixture, and the expected price of each day where the fixture has one.
    pub fn read(text: &str) -> (Vec<Bar>, Vec<f64>) {
        let mut bars = Vec::new();
        let mut expected = Vec::new();
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split(',').collect();
            let number = |index: usize, absent: f64| if fields[index].is_empty() { absent } else { fields[index].parse().unwrap() };
            bars.push(Bar { day: fields[0].parse().unwrap(), close: number(1, f64::NAN), dividend: number(2, 0.0), split: number(3, 1.0) });
            if let Some(price) = fields.get(4) {
                expected.push(price.parse().unwrap());
            }
        }
        (bars, expected)
    }

    pub fn bars(text: &str) -> Vec<Bar> {
        read(text).0
    }

    pub fn day(text: &str) -> Day {
        text.parse().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    /// How a rebuilt series compares with the expected one.
    struct Match {
        days: usize,
        /// Days on which the two prices are the same number.
        equal: usize,
        /// The largest gap between the two prices, as a share of the price.
        worst_level: f64,
        /// Days on which the two prices are within one part in 10,000.
        level_close: usize,
        /// Days on which the two daily changes are within 0.002%.
        change_close: usize,
    }

    fn compare(text: &str) -> Match {
        let (bars, expected) = read(text);
        let series = adjust(&bars).unwrap();
        let gap = |index: usize| (series.prices()[index] / expected[index] - 1.0).abs();
        let change = |prices: &[f64], index: usize| prices[index] / prices[index - 1];
        Match {
            days: bars.len(),
            equal: (0..bars.len()).filter(|&index| series.prices()[index] == expected[index]).count(),
            worst_level: (0..bars.len()).map(gap).fold(0.0, f64::max),
            level_close: (0..bars.len()).filter(|&index| gap(index) <= 1e-4).count(),
            change_close: (1..bars.len()).filter(|&index| (change(series.prices(), index) - change(&expected, index)).abs() <= 2e-5).count(),
        }
    }

    /// The precision reached: the same six-decimal price on 3,713 of 3,714
    /// days, through 59 dividends and a split. On the one other day the two
    /// sources print closes two cents apart.
    #[test]
    fn the_dividend_payer_matches_to_the_sixth_decimal() {
        let found = compare(DIVIDEND_PAYER);
        assert_eq!((found.equal, found.days), (3_713, 3_714));
        assert!(found.worst_level < 4e-4, "{}", found.worst_level);
    }

    /// The same six-decimal price on 3,656 of 3,714 days, through a 7-for-1
    /// and a 10-for-1 split. The other 58 are days the two sources print a
    /// different close: by half a cent or less on 56, where the expected
    /// series keeps a finer close, and by 22 and 28 cents on the other two.
    #[test]
    fn the_split_stock_matches_to_the_sixth_decimal() {
        let found = compare(SPLIT_STOCK);
        assert_eq!((found.equal, found.days), (3_656, 3_714));
        assert!(found.worst_level < 8e-4, "{}", found.worst_level);
        assert_eq!(found.change_close, 3_683);
    }

    /// A fund cannot be matched to the sixth decimal from public data, which
    /// gives its payouts to three decimals where the expected series was built
    /// from finer ones. What is reached: the daily change within 0.002% on
    /// 3,666 of 3,713 days, the level within 0.01% on 3,501 days and within
    /// 0.22% on all. The widest gap is the ten months before two payouts that
    /// the expected series counts at half as much again as was published.
    #[test]
    fn the_bond_fund_matches_as_far_as_its_payouts_are_known() {
        let found = compare(BOND_FUND);
        assert_eq!(found.days, 3_714);
        assert_eq!(found.change_close, 3_666);
        assert_eq!(found.level_close, 3_501);
        assert!(found.worst_level < 2.2e-3, "{}", found.worst_level);
    }

    /// Payouts to three decimals again. What is reached: the daily change
    /// within 0.002% on 3,598 of 3,713 days, the level within 0.01% on 3,709
    /// days and within 0.32% on all. The widest gap is one day on which the
    /// two sources' closes differ by 41 cents.
    #[test]
    fn the_index_fund_matches_as_far_as_its_payouts_are_known() {
        let found = compare(INDEX_FUND);
        assert_eq!(found.days, 3_714);
        assert_eq!(found.change_close, 3_598);
        assert_eq!(found.level_close, 3_709);
        assert!(found.worst_level < 3.2e-3, "{}", found.worst_level);
    }

    /// With no dividend and no split there is nothing to adjust: every day matches.
    #[test]
    fn the_new_listing_matches_on_every_day() {
        let found = compare(NEW_LISTING);
        assert_eq!((found.equal, found.days), (339, 339));
    }

    #[test]
    fn the_series_is_tied_to_the_last_real_close() {
        for text in [DIVIDEND_PAYER, SPLIT_STOCK, BOND_FUND, INDEX_FUND, NEW_LISTING] {
            let bars = bars(text);
            let series = adjust(&bars).unwrap();
            let last = bars.last().unwrap();
            assert_eq!(series.as_of(), last.day);
            assert_eq!(series.on(last.day), Some(last.close));
            // Back to the last dividend or split, the adjusted price is the close itself.
            let event = bars.iter().rposition(|bar| bar.dividend > 0.0 || bar.split != 1.0).unwrap_or(0);
            assert!(bars.len() - event > 5, "the fixture ends on an event");
            assert_eq!(series.prices()[event..], bars[event..].iter().map(|bar| bar.close).collect::<Vec<_>>());
        }
    }

    fn two_days(before: f64, close: f64, dividend: f64, split: f64) -> Series {
        let bars = [Bar::new(day("2026-01-05"), before), Bar { day: day("2026-01-06"), close, dividend, split }];
        adjust(&bars).unwrap()
    }

    fn change(series: &Series) -> f64 {
        series.prices()[1] / series.prices()[0] - 1.0
    }

    #[test]
    fn a_dividend_is_reinvested_on_its_ex_date() {
        // Three real ex-dates, with the change the expected series shows on each.
        // An ordinary quarterly dividend.
        let ordinary = two_days(89.35, 88.71, 0.53, 1.0);
        assert!((change(&ordinary) - -0.001_238_5).abs() < 2e-7, "{}", change(&ordinary));
        // A special dividend of 15 dollars: the close fell 1.16% and the holder was 1.09% up.
        let special = two_days(674.62, 666.80, 15.0, 1.0);
        assert!((change(&special) - 0.010_885_1).abs() < 2e-7, "{}", change(&special));
        // A dividend of a tenth of a percent.
        let small = two_days(223.67, 218.36, 0.25, 1.0);
        assert!((change(&small) - -0.022_647_9).abs() < 2e-7, "{}", change(&small));

        // The rule itself: the close over the close before less the dividend.
        assert!((change(&ordinary) - (88.71 / (89.35 - 0.53) - 1.0)).abs() < 2e-8);
        // Reinvesting at the ex-day close instead would give a different number.
        assert!((change(&ordinary) - ((88.71 + 0.53) / 89.35 - 1.0)).abs() > 5e-6);
        // The ex-day itself keeps its real close.
        assert_eq!(ordinary.prices()[1], 88.71);
    }

    #[test]
    fn a_split_leaves_no_jump() {
        // A real 10-for-1: the close fell 90.08% and the holder lost 0.83%.
        let forward = two_days(1_112.17, 110.29, 0.0, 10.0);
        assert_eq!(forward.prices(), [111.217, 110.29]);
        assert!((change(&forward) - -0.008_335).abs() < 1e-6, "{}", change(&forward));
        // A real 1-for-5: the close rose 435% and the holder gained 7.04%.
        let reverse = two_days(15.12, 80.92, 0.0, 0.2);
        assert_eq!(reverse.prices(), [75.6, 80.92]);
        assert!((change(&reverse) - 0.070_370).abs() < 1e-6, "{}", change(&reverse));
    }

    #[test]
    fn a_split_and_a_dividend_on_one_day_are_counted_in_the_same_shares() {
        // 100 dollars becomes two shares of 50. One dollar a new share goes ex
        // the same day and the close is 49: nothing gained and nothing lost.
        let both = two_days(100.0, 49.0, 1.0, 2.0);
        assert_eq!(both.prices(), [49.0, 49.0]);
    }

    #[test]
    fn every_later_event_reaches_back_to_every_earlier_day() {
        let mut bars: Vec<Bar> = (0..6).map(|index| Bar::new(Day::from_number(20_000 + index), 100.0)).collect();
        bars[2].dividend = 2.0;
        bars[4].split = 4.0;
        bars[4].close = 25.0;
        bars[5].close = 25.0;
        bars[5].dividend = 0.5;
        // The last dividend takes 2% off, the split divides by four, the first dividend takes another 2%.
        let prices = adjust(&bars).unwrap().prices().to_vec();
        assert_eq!(prices, [24.01, 24.01, 24.5, 24.5, 24.5, 25.0]);
    }

    #[test]
    fn prices_are_rounded_to_six_decimals_not_cut() {
        // 2 / 3 is 0.666666 cut off and 0.666667 rounded.
        let series = two_days(2.0, 1.0, 0.0, 3.0);
        assert_eq!(series.prices()[0], 0.666_667);
        // The first day of the split stock: 16.75 over 140 later shares.
        let early = two_days(16.75, 1.0, 0.0, 140.0);
        assert_eq!(early.prices()[0], 0.119_643);
    }

    #[test]
    fn a_price_is_found_by_day_and_by_index() {
        let bars = bars(DIVIDEND_PAYER);
        let series = adjust(&bars).unwrap();
        assert_eq!(series.len(), bars.len());
        assert_eq!((series.first_day(), series.last_day()), (day("2012-01-03"), day("2026-10-09")));
        for (index, bar) in bars.iter().enumerate() {
            assert_eq!(series.position(bar.day), Some(index));
            assert_eq!(series.on(bar.day), Some(series.prices()[index]));
            assert_eq!(series.days()[index], bar.day);
        }
        // A Saturday, a holiday, and days outside the history.
        for absent in ["2026-10-10", "2025-07-04", "2011-12-30", "2026-10-12"] {
            assert_eq!(series.on(day(absent)), None, "{absent}");
        }
    }

    #[test]
    fn bars_that_cannot_be_a_history_are_refused_with_a_reason() {
        let good = [Bar::new(day("2026-01-05"), 10.0), Bar::new(day("2026-01-06"), 11.0)];
        assert!(validate(&good).is_ok());
        let broken = |change: fn(&mut [Bar; 2])| {
            let mut bars = good;
            change(&mut bars);
            adjust(&bars).unwrap_err()
        };
        assert!(adjust(&[]).unwrap_err().contains("no days"));
        assert!(broken(|bars| bars[1].close = 0.0).contains("not a price"));
        assert!(broken(|bars| bars[0].close = f64::NAN).contains("not a price"));
        assert!(broken(|bars| bars[1].dividend = -0.1).contains("not an amount"));
        assert!(broken(|bars| bars[1].split = 0.0).contains("not a ratio"));
        assert!(broken(|bars| bars[1].day = bars[0].day).contains("out of order"));
        assert!(broken(|bars| bars.swap(0, 1)).contains("out of order"));
        assert!(broken(|bars| bars[1].dividend = 10.0).contains("as large as the close before it"));
    }
}
