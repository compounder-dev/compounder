//! The first screen's words and numbers as markup, so a page reads before any
//! script runs and with scripts switched off.
//!
//! The parts in `web/src` take this markup over in the browser as it stands, so
//! each function here writes exactly what its part draws. The pages kept in
//! `web/contract` hold both sides to that: the web build draws them with the
//! parts, and the tests below write them from here.

use compounder_engine::strategy::TradingSettings;
use serde_json::{json, Value};

use crate::{Holding, StarterView};

/// The screen an address belongs to. `matchRoute` in `web/src/route.ts` follows the same rules.
#[derive(Debug, PartialEq)]
pub enum Route {
    Home,
    Starter(String),
    NotFound,
}

pub fn route(path: &str) -> Route {
    if path == "/" {
        return Route::Home;
    }
    match path.strip_prefix("/s/") {
        Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') => Route::Starter(id.to_string()),
        _ => Route::NotFound,
    }
}

/// What a page shows, with the data it shows it from.
pub enum Screen<'a> {
    Home(&'a [StarterView]),
    Starter(&'a StarterView),
    NotFound,
    /// The server could not make the page that was asked for.
    Broke,
}

const ICON_HOME: &str = "M3.5 9.5 10 4l6.5 5.5V16a.5.5 0 0 1-.5.5h-3.5V12h-5v4.5H4a.5.5 0 0 1-.5-.5V9.5z";
const ICON_ARROW_LEFT: &str = "M16 10H4m5-5-5 5 5 5";
const ICON_CLOCK: &str = "M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zm0 3.5V10l2.5 2";

const FUND_COLOURS: [&str; 5] = ["blue", "orange", "teal", "violet", "magenta"];

/// Text made safe to sit in markup or in an attribute, with the same five replacements the browser side makes.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

fn icon(path: &str) -> String {
    format!(
        r#"<svg class="icon icon-md" viewBox="0 0 20 20" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="{path}"></path></svg>"#
    )
}

// ---- numbers: the rules of `web/src/format.ts`, in the same arithmetic ----

/// A whole number of `10^-decimals` units, halves rounded up. `amount` is never negative.
fn scaled(amount: f64, decimals: i32) -> u64 {
    (amount * 10f64.powi(decimals) + 0.5).floor() as u64
}

/// `units` of `10^-decimals` written out: 124018 at 2 becomes "1,240.18".
fn written(units: u64, decimals: usize) -> String {
    let digits = format!("{units:0>width$}", width = decimals + 1);
    let (whole, fraction) = digits.split_at(digits.len() - decimals);
    let mut out = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if decimals > 0 {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

fn signed(text: String, value: f64, always: bool) -> String {
    if value < 0.0 {
        format!("\u{2212}{text}")
    } else if always {
        format!("+{text}")
    } else {
        text
    }
}

/// A share or a change, given as a fraction: two decimals under 10, one from 10, none from 1,000.
pub fn percent(fraction: f64, sign: bool) -> String {
    let amount = fraction.abs() * 100.0;
    let (mut units, mut decimals) = (scaled(amount, 2), 2);
    if units >= 1000 {
        (units, decimals) = (scaled(amount, 1), 1);
        if units >= 10000 {
            (units, decimals) = (scaled(amount, 0), 0);
        }
    }
    if units == 0 {
        return "0%".to_string();
    }
    signed(format!("{}%", written(units, decimals)), fraction, sign)
}

/// A count or an amount: up to four decimals, and no zeros after the last digit that counts.
pub fn quantity(quantity: f64) -> String {
    let units = scaled(quantity.abs(), 4);
    let text = written(units, 4);
    let text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    if units == 0 {
        text
    } else {
        signed(text, quantity, false)
    }
}

fn number(text: &str) -> String {
    format!(r#"<span class="number">{text}</span>"#)
}

// ---- words drawn from a starter: the rules of `web/src/starter.ts` ----

/// The ticker a person recognises, without the issuer's ending.
fn ticker(holding: &Holding) -> String {
    let symbol = holding.symbol.clone().unwrap_or_else(|| holding.asset_id.chars().take(6).collect());
    symbol.strip_suffix("on").unwrap_or(&symbol).to_string()
}

/// The company or fund name, without a closing note in brackets.
fn display_name(holding: &Holding) -> &str {
    let text = holding.name.as_deref().or(holding.symbol.as_deref()).unwrap_or(&holding.asset_id);
    let trimmed = text.trim_end();
    let Some(before) = trimmed.strip_suffix(')') else { return text };
    // The note starts at the first opening bracket after any earlier closing one.
    let from = before.rfind(')').map_or(0, |index| index + 1);
    match before[from..].find('(') {
        Some(open) => before[..from + open].trim_end(),
        None => text,
    }
}

fn holds_line(holdings: &[Holding]) -> String {
    const SHOWN: usize = 4;
    // One ticker is never left as "and 1 more" when it could just be named.
    let shown = if holdings.len() <= SHOWN + 1 { holdings } else { &holdings[..SHOWN] };
    let names = shown.iter().map(ticker).collect::<Vec<_>>().join(", ");
    match holdings.len() - shown.len() {
        0 => names,
        rest => format!("{names} and {rest} more"),
    }
}

fn trades_line(settings: &TradingSettings) -> String {
    match settings {
        TradingSettings::Threshold { trigger_percentage } => format!("When a holding drifts {}% from its target", quantity(*trigger_percentage)),
        TradingSettings::Calendar { frequency } => format!("On a {frequency} schedule"),
    }
}

/// The identity colour a starter was given, or the first one for a name outside the five.
fn fund_colour(accent: &str) -> &'static str {
    FUND_COLOURS.iter().copied().find(|colour| *colour == accent).unwrap_or(FUND_COLOURS[0])
}

/// A holding's bar beside the largest holding's, as a whole number from 0 to 100.
fn bar_share(weight: f64, largest: f64) -> u64 {
    if largest > 0.0 {
        ((weight / largest) * 100.0 + 0.5).floor() as u64
    } else {
        0
    }
}

// ---- the screens ----

fn home(starters: &[StarterView]) -> String {
    let mut out = String::from(
        r#"<h1 class="page-title home-title">Choose your fund</h1><p class="page-lead">Start from a fund that is already built. Open one to see what it holds and when it trades.</p>"#,
    );
    if starters.is_empty() {
        out.push_str(r#"<div class="region-notice"><div class="empty"><p>There are no funds to choose from yet.</p><button type="button" class="button button-secondary">Look again</button></div></div>"#);
        return out;
    }
    out.push_str(r#"<ul class="cards" role="list">"#);
    for starter in starters {
        out.push_str(&format!(
            concat!(
                r#"<li class="card card-strategy fund-{colour}">"#,
                r#"<h2 class="card-title"><span class="fund-dot" aria-hidden="true"></span><a class="card-link" href="/s/{id}">{name}</a></h2>"#,
                r#"<p class="card-summary">{summary}</p>"#,
                r#"<dl class="card-facts"><div><dt>Holds</dt><dd>{holds}</dd></div></dl></li>"#,
            ),
            colour = fund_colour(&starter.accent),
            id = escape(&starter.id),
            name = escape(&starter.name),
            summary = escape(&starter.summary),
            holds = escape(&holds_line(&starter.holdings)),
        ));
    }
    out.push_str("</ul>");
    out
}

fn starter(starter: &StarterView) -> String {
    let largest = starter.holdings.iter().map(|holding| holding.weight).fold(0.0, f64::max);
    let mut out = format!(
        concat!(
            r#"<p class="back-link"><a class="link link-standalone" href="/">{back}All funds</a></p>"#,
            r#"<div class="starter fund-{colour}">"#,
            r#"<header class="starter-head"><h1 class="page-title"><span class="fund-dot" aria-hidden="true"></span>{name}</h1><p class="page-lead">{summary}</p></header>"#,
            r#"<aside class="card card-plain starter-facts" aria-label="Facts"><dl class="facts">"#,
            r#"<div><dt>Holdings</dt><dd>{count}</dd></div><div><dt>Trades</dt><dd>{trades}</dd></div></dl>"#,
            r#"<p class="starter-note">Adding money is not open yet.</p></aside>"#,
            r#"<section class="starter-holdings" aria-labelledby="holdings-title"><h2 class="section-title" id="holdings-title">What it holds</h2><ul class="rows" role="list">"#,
        ),
        back = icon(ICON_ARROW_LEFT),
        colour = fund_colour(&starter.accent),
        name = escape(&starter.name),
        summary = escape(&starter.summary),
        count = number(&quantity(starter.holdings.len() as f64)),
        trades = escape(&trades_line(&starter.strategy.trading_settings)),
    );
    for holding in &starter.holdings {
        let detail = holding.symbol.as_deref().map(|symbol| format!(r#"<span class="row-detail">{}</span>"#, escape(symbol))).unwrap_or_default();
        out.push_str(&format!(
            concat!(
                r#"<li class="row row-asset"><span class="mark" aria-hidden="true">{letter}</span>"#,
                r#"<span class="row-text"><span class="row-title">{name}</span>{detail}</span>"#,
                r#"<span class="row-trailing"><span class="bar" aria-hidden="true"><span style="--share:{share}"></span></span>{weight}</span></li>"#,
            ),
            letter = escape(&ticker(holding).chars().take(1).collect::<String>()),
            name = escape(display_name(holding)),
            detail = detail,
            share = bar_share(holding.weight, largest),
            weight = number(&percent(holding.weight, false)),
        ));
    }
    // The line about the check in the browser holds its place and says nothing until a browser has run it.
    out.push_str(&format!(
        r#"</ul><p class="check check-idle" role="status">{}Checking the allocation in your browser…</p></section></div>"#,
        icon(ICON_CLOCK)
    ));
    out
}

const NOT_FOUND: &str = r#"<div class="system"><h1 class="page-title">That page does not exist.</h1><a class="button button-primary" href="/">Go home</a></div>"#;

fn broke(path: &str) -> String {
    let home = if path == "/" { "" } else { r#"<a class="button button-ghost" href="/">Go home</a>"# };
    format!(
        r#"<div class="system" role="alert"><h1 class="page-title">Something went wrong on our side.</h1><div class="system-actions"><a class="button button-primary" href="{}">Try again</a>{home}</div></div>"#,
        escape(path)
    )
}

/// The name of the page in the tab and in the history list.
pub fn title(screen: &Screen) -> String {
    match screen {
        Screen::Home(_) => "Compounder".to_string(),
        Screen::Starter(starter) => format!("{} · Compounder", starter.name),
        Screen::NotFound => "Page not found · Compounder".to_string(),
        Screen::Broke => "Something went wrong · Compounder".to_string(),
    }
}

/// Everything inside the page's root: the frame every screen stands in, and the screen.
pub fn body(path: &str, screen: &Screen) -> String {
    // The Home tab is marked on the home screen itself and on the screens under it.
    let current = match route(path) {
        Route::Home => r#" aria-current="page""#,
        Route::Starter(_) => r#" aria-current="true""#,
        Route::NotFound => "",
    };
    let inside = match screen {
        Screen::Home(starters) => home(starters),
        Screen::Starter(view) => starter(view),
        Screen::NotFound => NOT_FOUND.to_string(),
        Screen::Broke => broke(path),
    };
    format!(
        concat!(
            r##"<a class="skip-link" href="#main">Skip to content</a><div class="offline-bar" role="status"></div>"##,
            r#"<header class="top-bar"><div class="top-bar-inner"><a class="wordmark" href="/" aria-label="Compounder, home"><span>Compounder</span>"#,
            r#"<svg class="wordmark-mark" viewBox="0 0 32 32" width="24" height="24" aria-hidden="true"><path d="M5 25c8 0 12.5-2.4 15.7-7.2S25 9 26 5" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"></path><circle cx="26" cy="5" r="3" fill="currentColor"></circle></svg></a>"#,
            r#"<nav class="top-bar-nav" aria-label="Main"><a class="top-bar-link" href="/"{current}>Home</a></nav></div></header>"#,
            r#"<main id="main" class="page" tabindex="-1">{inside}</main>"#,
            r#"<nav class="bottom-bar" aria-label="Main"><a class="tab" href="/"{current}><span class="tab-mark">{home}</span><span>Home</span></a></nav>"#,
        ),
        current = current,
        inside = inside,
        home = icon(ICON_HOME),
    )
}

/// The answers the page's first screen would otherwise ask for, keyed by the path it would ask.
/// A null says the server has no such thing.
pub fn seed(path: &str, screen: &Screen) -> Value {
    let resources = match (screen, route(path)) {
        (Screen::Home(starters), _) => json!({ "/v1/starters": { "starters": starters } }),
        (Screen::Starter(view), _) => json!({ format!("/v1/starters/{}", view.id): { "starter": view } }),
        (Screen::NotFound, Route::Starter(id)) => json!({ format!("/v1/starters/{id}"): null }),
        _ => json!({}),
    };
    match screen {
        Screen::Broke => json!({ "resources": resources, "broke": true }),
        _ => json!({ "resources": resources }),
    }
}

pub const TITLE_PLACE: &str = "<title>Compounder</title>";
pub const PAGE_PLACE: &str = "<!--page-->";
pub const SEED_PLACE: &str = "<!--seed-->";

/// The page with nothing but its places, for when the web app has not been built.
/// The words still arrive. They arrive without their styles.
pub const BARE_SHELL: &str = concat!(
    r#"<!doctype html><html lang="en"><head><meta charset="UTF-8" /><meta name="viewport" content="width=device-width, initial-scale=1.0" />"#,
    r#"<title>Compounder</title></head><body><div id="root"><!--page--></div><!--seed--></body></html>"#,
);

/// The built shell when it has every place a page is written into, and the bare one when it does not:
/// the web app is not built, or was built before pages were written here. The words arrive either way.
pub fn shell_or_bare(built: &str) -> &str {
    if [TITLE_PLACE, PAGE_PLACE, SEED_PLACE].iter().all(|place| built.contains(place)) {
        built
    } else {
        BARE_SHELL
    }
}

/// A whole page: the web app's shell with the title, the screen and its data written into their places.
pub fn document(shell: &str, path: &str, screen: &Screen) -> String {
    // The data sits in a script element, so nothing in it may look like the end of one.
    let data = seed(path, screen).to_string().replace('<', "\\u003c");
    shell
        .replacen(TITLE_PLACE, &format!("<title>{}</title>", escape(&title(screen))), 1)
        .replacen(SEED_PLACE, &format!(r#"<script type="application/json" id="seed">{data}</script>"#), 1)
        .replacen(PAGE_PLACE, &body(path, screen), 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn contract(file: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/contract").join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    }

    fn cases(file: &str) -> Vec<Value> {
        serde_json::from_str(&contract(file)).expect("the contract file is valid")
    }

    /// What a kept page shows, read back from the answers its case carries.
    fn body_for(case: &Value) -> String {
        let address = case["address"].as_str().unwrap();
        if case["broke"] == true {
            return body(address, &Screen::Broke);
        }
        match route(address) {
            Route::Home => {
                let starters: Vec<StarterView> = serde_json::from_value(case["resources"]["/v1/starters"]["starters"].clone()).unwrap();
                body(address, &Screen::Home(&starters))
            }
            Route::Starter(id) => match &case["resources"][format!("/v1/starters/{id}")] {
                Value::Null => body(address, &Screen::NotFound),
                found => {
                    let view: StarterView = serde_json::from_value(found["starter"].clone()).unwrap();
                    body(address, &Screen::Starter(&view))
                }
            },
            Route::NotFound => body(address, &Screen::NotFound),
        }
    }

    #[test]
    fn every_kept_page_is_written_here_exactly_as_the_parts_draw_it() {
        for case in cases("cases.json") {
            let name = case["name"].as_str().unwrap();
            let kept = contract(&format!("{name}.html"));
            let written = body_for(&case) + "\n";
            if written != kept {
                let at = written.bytes().zip(kept.bytes()).take_while(|(a, b)| a == b).count();
                let around = |text: &str| String::from_utf8_lossy(&text.as_bytes()[at.saturating_sub(60)..(at + 60).min(text.len())]).into_owned();
                panic!("{name}: differs at byte {at}\n  kept:    {}\n  written: {}", around(&kept), around(&written));
            }
        }
    }

    #[test]
    fn addresses_match_by_the_rules_the_browser_follows() {
        for case in cases("routes.json") {
            let expected = match case["screen"].as_str().unwrap() {
                "home" => Route::Home,
                "starter" => Route::Starter(case["id"].as_str().unwrap().to_string()),
                _ => Route::NotFound,
            };
            assert_eq!(route(case["path"].as_str().unwrap()), expected, "{}", case["path"]);
        }
    }

    #[test]
    fn numbers_print_as_the_browser_prints_them() {
        let mut checked = 0;
        for case in cases("numbers.json") {
            let value = case["value"].as_f64().unwrap();
            let sign = case["sign"] == true;
            let printed = match case["kind"].as_str().unwrap() {
                "percent" => percent(value, sign),
                "quantity" => quantity(value),
                // The server prints no money, ratios or countdowns yet.
                _ => continue,
            };
            assert_eq!(printed, case["text"].as_str().unwrap(), "{case}");
            checked += 1;
        }
        assert!(checked > 20);
    }

    #[test]
    fn a_closing_note_in_brackets_is_dropped_from_a_name() {
        let named = |name: &str| Holding { asset_id: "id".into(), symbol: None, name: Some(name.into()), weight: 0.0 };
        assert_eq!(display_name(&named("Gold Trust (Tokenized)")), "Gold Trust");
        assert_eq!(display_name(&named("Class A (b (c)  ")), "Class A");
        assert_eq!(display_name(&named("Open (a) b)")), "Open (a) b)");
        assert_eq!(display_name(&named("(Only)")), "");
        assert_eq!(display_name(&named("Plain ")), "Plain ");
    }

    #[test]
    fn the_data_in_a_page_cannot_close_its_own_script() {
        let mut view: StarterView = serde_json::from_value(cases("cases.json")[2]["resources"]["/v1/starters/six-makers"]["starter"].clone()).unwrap();
        view.summary = "</script><script>alert(1)</script>".into();
        let page = document(BARE_SHELL, "/s/six-makers", &Screen::Starter(&view));
        assert_eq!(page.matches("</script>").count(), 1, "{page}");
        let data = page.split(r#"id="seed">"#).nth(1).unwrap().split("</script>").next().unwrap();
        let read: Value = serde_json::from_str(data).unwrap();
        assert_eq!(read["resources"]["/v1/starters/six-makers"]["starter"]["summary"], "</script><script>alert(1)</script>");
    }

    #[test]
    fn the_web_app_shell_has_the_places_this_writes_into() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/index.html");
        let shell = std::fs::read_to_string(path).unwrap();
        for place in [TITLE_PLACE, PAGE_PLACE, SEED_PLACE] {
            assert_eq!(shell.matches(place).count(), 1, "{place}");
            assert_eq!(BARE_SHELL.matches(place).count(), 1, "{place}");
        }
    }
}
