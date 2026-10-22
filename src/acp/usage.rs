//! Plan limits of Claude Code (5-hour and weekly window). The ACP adapter
//! answers a prompt that is exactly `/usage` locally – no model request, no
//! tokens – with Markdown like
//! `**5-hour limit** — **12%** · Resets Oct 19, 10:10 PM GMT+2`.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// Label as sent ("5-hour limit", "Weekly · all models" …).
    pub label: String,
    /// Used share in percent.
    pub used: f32,
    pub reset: Option<Reset>,
}

/// When a window starts over, in the time zone the agent printed it in.
#[derive(Clone, Debug, PartialEq)]
pub struct Reset {
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// Unix time (seconds) of the reset.
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Plan with provider ("Claude pro", "ChatGPT plus" …) if given.
    pub plan: Option<String>,
    pub windows: Vec<Window>,
    /// When the numbers were read (unix seconds).
    pub fetched: i64,
}

impl Limits {
    pub fn five_hour(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.label.starts_with("5-hour"))
    }

    pub fn weekly(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.label.starts_with("Weekly") && w.label.contains("all"))
            .or_else(|| self.windows.iter().find(|w| w.label.starts_with("Weekly")))
    }
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Reads the limits from the `/usage` answer; `None` if it has none.
pub fn parse(markdown: &str, now: i64) -> Option<Limits> {
    let plan = markdown
        .lines()
        .find_map(|l| l.trim().strip_prefix("> Claude ")?.strip_suffix(" subscription usage").map(|p| format!("Claude {p}")));
    let windows: Vec<Window> = markdown.lines().filter_map(|l| window(l.trim(), now)).collect();
    (!windows.is_empty()).then_some(Limits { plan, windows, fetched: now })
}

/// Reads Codex's `/status` answer (also local, no tokens), with lines like
/// `**codex 5h limit:** 73% left (resets 14:20)` and
/// `**codex Weekly limit:** 90% left (resets 09:00 on 25 Oct)` – times in
/// local time, the date in the system's language (only the day is used).
/// `offset_min`: minutes local time is ahead of UTC.
pub fn parse_codex(markdown: &str, now: i64, offset_min: i64) -> Option<Limits> {
    let mut plan = None;
    let mut windows = Vec::new();
    for line in markdown.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("**") else { continue };
        let Some((label, rest)) = rest.split_once(":**") else { continue };
        if label == "Account" {
            plan = rest.trim().strip_prefix("ChatGPT ").and_then(|r| r.split_whitespace().next()).map(|p| format!("ChatGPT {p}"));
            continue;
        }
        let lower = label.to_lowercase();
        if !lower.ends_with("limit") || lower.contains("spend") {
            continue;
        }
        let Some((left, rest)) = rest.trim().split_once("% left") else { continue };
        let Ok(left) = left.trim().parse::<f32>() else { continue };
        // Same names as Claude's, so the panel shows both alike.
        let label = if lower.contains("5h") {
            "5-hour limit".to_string()
        } else if lower.contains("weekly") || lower.contains("7d") {
            "Weekly limit".to_string()
        } else {
            label.to_string()
        };
        let reset = rest.split_once("(resets ").and_then(|(_, r)| codex_reset(r.trim_end_matches(')').trim(), now, offset_min));
        windows.push(Window { label, used: (100.0 - left).clamp(0.0, 100.0), reset });
    }
    (!windows.is_empty()).then_some(Limits { plan, windows, fetched: now })
}

/// `14:20` (today) or `09:00 on 25 Oct` / `09:00 on 25. Okt.` (the next 25th).
fn codex_reset(s: &str, now: i64, offset_min: i64) -> Option<Reset> {
    let (time, date) = s.split_once(" on ").map_or((s, None), |(t, d)| (t, Some(d)));
    let (h, m) = time.trim().split_once(':')?;
    let (hour, minute): (u32, u32) = (h.trim().parse().ok()?, m.trim().get(..2)?.parse().ok()?);
    let local_days = (now + offset_min * 60).div_euclid(86_400);
    let days = match date {
        None => local_days,
        Some(d) => {
            let day: u32 = d.split(|c: char| !c.is_ascii_digit()).find(|p| !p.is_empty())?.parse().ok()?;
            (local_days..local_days + 32).find(|&n| civil_from_days(n).2 == day)?
        }
    };
    let (_, month, day) = civil_from_days(days);
    let at = days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 - offset_min * 60;
    Some(Reset { month, day, hour, minute, at })
}

/// `**Label** — **12%** · Resets Oct 19, 10:10 PM GMT+2`
fn window(line: &str, now: i64) -> Option<Window> {
    let rest = line.strip_prefix("**")?;
    let (label, rest) = rest.split_once("**")?;
    let rest = rest.trim_start().strip_prefix('—')?.trim_start().strip_prefix("**")?;
    let (pct, rest) = rest.split_once("%**")?;
    let used: f32 = pct.trim().parse().ok()?;
    let reset = rest.trim().strip_prefix("· Resets ").and_then(|r| parse_reset(r.trim(), now));
    Some(Window { label: label.to_string(), used, reset })
}

/// `Oct 19, 10:10 PM GMT+2` (English, year left out: the next such date from now).
fn parse_reset(s: &str, now: i64) -> Option<Reset> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = s.split_whitespace();
    let name = parts.next()?;
    let month = MONTHS.iter().position(|m| *m == name)? as u32 + 1;
    let day: u32 = parts.next()?.trim_end_matches(',').parse().ok()?;
    let (h, m) = parts.next()?.split_once(':')?;
    let (mut hour, minute): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    match parts.next()? {
        "PM" if hour != 12 => hour += 12,
        "AM" if hour == 12 => hour = 0,
        "AM" | "PM" => {}
        _ => return None,
    }
    // "GMT+2", "GMT-5:30", "UTC", "GMT".
    let offset_min = match parts.next() {
        Some(tz) => {
            let off = tz.trim_start_matches("GMT").trim_start_matches("UTC");
            if off.is_empty() {
                0
            } else {
                let sign = if off.starts_with('-') { -1 } else { 1 };
                let off = off.trim_start_matches(['+', '-']);
                let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
                sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?)
            }
        }
        None => 0,
    };
    let year_now = civil_from_days(now.div_euclid(86_400)).0;
    let at_year = |y: i64| days_from_civil(y, month, day) * 86_400 + hour as i64 * 3600 + minute as i64 * 60 - offset_min * 60;
    // The reset lies in the future (allow a little lag); around new year it is next year's date.
    let mut at = at_year(year_now);
    if at < now - 86_400 {
        at = at_year(year_now + 1);
    }
    Some(Reset { month, day, hour, minute, at })
}

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// "2 Std. 13 Min." / "3 Tg. 4 Std." until `at`.
pub fn until(at: i64, now: i64) -> String {
    let mins = ((at - now).max(0) + 59) / 60;
    let (d, h, m) = (mins / 1440, mins / 60 % 24, mins % 60);
    if d > 0 {
        t!("{d} Tg. {h} Std.", d = d, h = h)
    } else if h > 0 {
        t!("{h} Std. {m} Min.", h = h, m = m)
    } else {
        t!("{m} Min.", m = m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "## Usage\n\n> Claude pro subscription usage\n\n### Limits\n\n\
        **5-hour limit** — **12%** · Resets Oct 19, 10:10 PM GMT+2\n\n`██░░`\n\n\
        **Weekly · all models** — **88%** · Resets Oct 22, 3:00 AM GMT+2\n\n`██████`\n\n---\n\n### This session\n";

    #[test]
    fn reads_the_usage_answer() {
        // 2026-10-19 18:00 UTC
        let now = days_from_civil(2026, 10, 19) * 86_400 + 18 * 3600;
        let l = parse(SAMPLE, now).unwrap();
        assert_eq!(l.plan.as_deref(), Some("Claude pro"));
        let five = l.five_hour().unwrap();
        assert_eq!(five.used, 12.0);
        let r = five.reset.as_ref().unwrap();
        assert_eq!((r.month, r.day, r.hour, r.minute), (10, 19, 22, 10));
        // 22:10 at GMT+2 is 20:10 UTC: 2 h 10 min after 18:00 UTC.
        assert_eq!(r.at - now, 2 * 3600 + 10 * 60);
        assert_eq!(until(r.at, now), "2 Std. 10 Min.");
        let week = l.weekly().unwrap();
        assert_eq!(week.used, 88.0);
        assert_eq!(week.reset.as_ref().unwrap().hour, 3);
        assert!(parse("Hallo", now).is_none());
    }

    #[test]
    fn reads_the_codex_status() {
        let status = "**Model:** gpt-5.5  \n**Account:** ChatGPT plus (a@b.de)  \n\n**Token usage:** 1.2K total  \n\
                      **codex 5h limit:** 73% left (resets 14:20)  \n**codex Weekly limit:** 90% left (resets 09:00 on 25. Okt.)";
        // 2026-10-22 10:00 local time at UTC+2 = 08:00 UTC.
        let now = days_from_civil(2026, 10, 22) * 86_400 + 8 * 3600;
        let l = parse_codex(status, now, 120).unwrap();
        assert_eq!(l.plan.as_deref(), Some("ChatGPT plus"));
        let five = l.five_hour().unwrap();
        assert_eq!(five.used, 27.0);
        let r = five.reset.as_ref().unwrap();
        assert_eq!((r.day, r.hour, r.minute), (22, 14, 20));
        assert_eq!(r.at - now, 4 * 3600 + 20 * 60);
        let week = l.weekly().unwrap();
        assert_eq!(week.used, 10.0);
        let w = week.reset.as_ref().unwrap();
        assert_eq!((w.month, w.day, w.hour), (10, 25, 9));
        assert!(parse_codex("**Limits:** data not available yet", now, 120).is_none());
    }

    #[test]
    fn dates_round_trip() {
        for days in [0, 18_000, 20_745, 21_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }
}
