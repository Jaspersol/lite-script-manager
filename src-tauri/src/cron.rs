use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct CronSpec {
    seconds: Vec<bool>,
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days: Vec<bool>,
    months: Vec<bool>,
    weekdays: Vec<bool>,
    day_any: bool,
    weekday_any: bool,
}

fn parse_value(value: &str, names: &[&str], min: u32, max: u32) -> Result<u32, String> {
    let upper = value.to_ascii_uppercase();
    if let Some(index) = names.iter().position(|name| *name == upper) {
        return Ok(min + index as u32);
    }
    let number = value
        .parse::<u32>()
        .map_err(|_| format!("无法解析时间字段: {value}"))?;
    if number < min || number > max {
        return Err(format!("时间字段超出范围: {value}"));
    }
    Ok(number)
}

fn parse_field(
    field: &str,
    min: u32,
    max: u32,
    names: &[&str],
    normalize_sunday: bool,
) -> Result<(Vec<bool>, bool), String> {
    let mut values = vec![false; (max - min + 1) as usize];
    let mut any = false;
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("Cron 字段为空".into());
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (
                range,
                step.parse::<u32>()
                    .map_err(|_| format!("无法解析步长: {step}"))?,
            ),
            None => (part, 1),
        };
        if step == 0 {
            return Err("Cron 步长必须大于 0".into());
        }
        let (start, end) = if range == "*" {
            if step == 1 {
                any = true;
            }
            (min, max)
        } else if let Some((start, end)) = range.split_once('-') {
            (
                parse_value(start, names, min, max)?,
                parse_value(end, names, min, max)?,
            )
        } else {
            let value = parse_value(range, names, min, max)?;
            if part.contains('/') {
                (value, max)
            } else {
                (value, value)
            }
        };
        if start > end {
            return Err(format!("Cron 范围起点大于终点: {range}"));
        }
        let mut value = start;
        while value <= end {
            let index = if normalize_sunday && value == 7 {
                0
            } else {
                value - min
            };
            if index < values.len() as u32 {
                values[index as usize] = true;
            }
            value += step;
        }
    }
    Ok((values, any))
}

impl CronSpec {
    pub fn parse(expression: &str) -> Result<Self, String> {
        let fields = expression.split_whitespace().collect::<Vec<_>>();
        let fields = match fields.len() {
            5 => {
                let mut values = vec!["0"];
                values.extend(fields);
                values
            }
            6 => fields,
            _ => {
                return Err(
                    "Cron 表达式需要 5 段（分 时 日 月 周）或 6 段（秒 分 时 日 月 周）".into(),
                );
            }
        };
        let (seconds, _) = parse_field(fields[0], 0, 59, &[], false)?;
        let (minutes, _) = parse_field(fields[1], 0, 59, &[], false)?;
        let (hours, _) = parse_field(fields[2], 0, 23, &[], false)?;
        let (days, day_any) = parse_field(fields[3], 1, 31, &[], false)?;
        let (months, _) = parse_field(
            fields[4],
            1,
            12,
            &[
                "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
            ],
            false,
        )?;
        let (weekdays, weekday_any) = parse_field(
            fields[5],
            0,
            7,
            &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"],
            true,
        )?;
        let spec = Self {
            seconds,
            minutes,
            hours,
            days,
            months,
            weekdays,
            day_any,
            weekday_any,
        };
        if !spec.has_possible_date() {
            return Err("Cron 表达式不会匹配任何日期（例如 2 月 30 日）".into());
        }
        Ok(spec)
    }

    /// A 400-year window covers every month/day/weekday combination of the
    /// Gregorian calendar, so a bounded scan is exact. This rejects expressions
    /// like `0 0 30 2 *` before they burn iterations on every computation.
    fn has_possible_date(&self) -> bool {
        if self.day_any && self.weekday_any {
            return !self.months.iter().all(|value| !value);
        }
        let mut date = NaiveDate::from_ymd_opt(2000, 1, 1);
        let end = NaiveDate::from_ymd_opt(2400, 1, 1);
        while let Some(current) = date {
            if Some(current) >= end {
                break;
            }
            if self.months[(current.month() - 1) as usize] && self.day_matches(current) {
                return true;
            }
            date = current.succ_opt();
        }
        false
    }

    fn day_matches(&self, date: NaiveDate) -> bool {
        let dom = self.days[(date.day() - 1) as usize];
        let dow = self.weekdays[date.weekday().num_days_from_sunday() as usize];
        match (self.day_any, self.weekday_any) {
            (true, true) => true,
            (false, false) => dom || dow,
            (false, true) => dom,
            (true, false) => dow,
        }
    }
}

fn next_in_zone<TzImpl: TimeZone>(
    spec: &CronSpec,
    after: DateTime<Utc>,
    zone: &TzImpl,
) -> Result<i64, String> {
    let mut local = after.with_timezone(zone).naive_local();
    local += chrono::Duration::seconds(1);
    local = local.with_nanosecond(0).unwrap_or(local);
    for _ in 0..3_000_000 {
        if !spec.months[(local.month() - 1) as usize] {
            let year = local.year();
            let month = local.month();
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            local = NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .ok_or_else(|| "Cron 计算日期溢出".to_string())?
                .and_hms_opt(0, 0, 0)
                .unwrap();
            continue;
        }
        if !spec.day_matches(local.date()) {
            local = local
                .date()
                .succ_opt()
                .ok_or_else(|| "Cron 计算日期溢出".to_string())?
                .and_hms_opt(0, 0, 0)
                .unwrap();
            continue;
        }
        if !spec.hours[local.hour() as usize] {
            local = local
                .with_minute(0)
                .and_then(|value| value.with_second(0))
                .unwrap_or(local)
                + chrono::Duration::hours(1);
            continue;
        }
        if !spec.minutes[local.minute() as usize] {
            local = local.with_second(0).unwrap_or(local) + chrono::Duration::minutes(1);
            continue;
        }
        if !spec.seconds[local.second() as usize] {
            local += chrono::Duration::seconds(1);
            continue;
        }
        let after_ms = after.timestamp_millis();
        match zone.from_local_datetime(&local) {
            chrono::LocalResult::Single(value) => {
                if value.timestamp_millis() > after_ms {
                    return Ok(value.timestamp_millis());
                }
                local += chrono::Duration::seconds(1);
            }
            // Fall-back repeats the local hour: pick the earliest occurrence
            // strictly after `after`, never one that is already in the past.
            chrono::LocalResult::Ambiguous(first, second) => {
                let candidate = if first.timestamp_millis() > after_ms {
                    Some(first)
                } else if second.timestamp_millis() > after_ms {
                    Some(second)
                } else {
                    None
                };
                if let Some(value) = candidate {
                    return Ok(value.timestamp_millis());
                }
                local += chrono::Duration::seconds(1);
            }
            // Spring-forward gaps have no matching local time; step over the
            // gap minute by minute instead of trusting a single conversion.
            chrono::LocalResult::None => {
                local += chrono::Duration::minutes(1);
            }
        }
        if local.year() > after.with_timezone(zone).year() + 8 {
            break;
        }
    }
    Err("未来 8 年内没有匹配的执行时间".into())
}

pub fn parse_timezone(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("local") {
        return Ok("local".into());
    }
    if value.eq_ignore_ascii_case("utc") {
        return Ok("UTC".into());
    }
    Tz::from_str(value)
        .map(|_| value.to_string())
        .map_err(|_| format!("未知时区: {value}"))
}

pub fn next_cron_after(expression: &str, timezone: &str, after_ms: i64) -> Result<i64, String> {
    let spec = CronSpec::parse(expression)?;
    let zone = parse_timezone(timezone)?;
    let after = Utc
        .timestamp_millis_opt(after_ms)
        .single()
        .ok_or_else(|| "时间戳无效".to_string())?;
    if zone == "local" {
        next_in_zone(&spec, after, &Local)
    } else {
        let tz = Tz::from_str(&zone).map_err(|_| format!("未知时区: {zone}"))?;
        next_in_zone(&spec, after, &tz)
    }
}

pub fn parse_interval(value: &str) -> Result<i64, String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err("间隔不能为空".into());
    }
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else if let Some(value) = value.strip_suffix('h') {
        (value, 3_600_000)
    } else if let Some(value) = value.strip_suffix('d') {
        (value, 86_400_000)
    } else {
        (value.as_str(), 1000)
    };
    let number = number
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("无法解析间隔: {value}"))?;
    if number <= 0 {
        return Err("间隔必须大于 0".into());
    }
    Ok(number.saturating_mul(multiplier).max(1000))
}

pub fn parse_once(value: &str) -> Result<i64, String> {
    let value = value.trim();
    if let Ok(timestamp) = value.parse::<i64>() {
        return Ok(if timestamp < 10_000_000_000 {
            timestamp * 1000
        } else {
            timestamp
        });
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        return Ok(value.timestamp_millis());
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(Local
                .from_local_datetime(&value)
                .earliest()
                .unwrap_or_else(|| Local.from_utc_datetime(&value))
                .timestamp_millis());
        }
    }
    Err("无法解析执行时间，请使用 2026-01-01 12:00 或 RFC3339".into())
}

pub fn next_run_ms(
    kind: &str,
    expression: &str,
    timezone: &str,
    after_ms: i64,
) -> Result<i64, String> {
    match kind {
        "cron" => next_cron_after(expression, timezone, after_ms),
        "interval" => Ok(after_ms + parse_interval(expression)?),
        "once" => parse_once(expression),
        "loop" => Ok(after_ms + parse_interval(expression)?),
        other => Err(format!("未知计划类型: {other}")),
    }
}

pub fn preview(
    kind: &str,
    expression: &str,
    timezone: &str,
    after_ms: i64,
    count: usize,
) -> Result<Vec<i64>, String> {
    let mut values = Vec::new();
    let mut cursor = after_ms;
    for _ in 0..count.clamp(1, 10) {
        let next = next_run_ms(kind, expression, timezone, cursor)?;
        values.push(next);
        cursor = next;
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> i64 {
        NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    fn utc(value: &str) -> i64 {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn parses_five_and_six_fields() {
        assert!(CronSpec::parse("*/5 * * * *").is_ok());
        assert!(CronSpec::parse("0 */5 * * * *").is_ok());
        assert!(CronSpec::parse("bad").is_err());
    }

    #[test]
    fn finds_next_cron_minute() {
        let next = next_cron_after("*/5 * * * *", "utc", at("2026-01-01 12:02:30")).unwrap();
        assert_eq!(next, at("2026-01-01 12:05:00"));
    }

    #[test]
    fn finds_next_cron_second() {
        let next = next_cron_after("*/10 * * * * *", "utc", at("2026-01-01 12:02:31")).unwrap();
        assert_eq!(next, at("2026-01-01 12:02:40"));
    }

    #[test]
    fn parses_interval_suffixes() {
        assert_eq!(parse_interval("5m").unwrap(), 300_000);
        assert_eq!(parse_interval("30").unwrap(), 30_000);
    }

    #[test]
    fn rejects_impossible_calendar_dates() {
        assert!(CronSpec::parse("0 0 30 2 *").is_err());
        assert!(CronSpec::parse("0 0 31 4 *").is_err());
        assert!(CronSpec::parse("0 0 31 2,4,6,9,11 *").is_err());
        // Leap day exists, so this one is valid.
        assert!(CronSpec::parse("0 0 29 2 *").is_ok());
        assert!(CronSpec::parse("0 0 * * *").is_ok());
        assert!(CronSpec::parse("0 0 * * MON").is_ok());
        assert!(next_cron_after("0 0 30 2 *", "utc", at("2026-01-01 00:00:00")).is_err());
    }

    #[test]
    fn fall_back_never_returns_a_past_occurrence() {
        // 2026-11-01 in New York: 01:30 happens at 05:30Z (EDT) and again at
        // 06:30Z (EST). From 06:15Z the next run must be the second one.
        let after = utc("2026-11-01T06:15:00Z");
        let next = next_cron_after("30 1 * * *", "America/New_York", after).unwrap();
        assert_eq!(next, utc("2026-11-01T06:30:00Z"));
        assert!(next > after);
    }

    #[test]
    fn fall_back_picks_the_first_occurrence_when_it_is_still_ahead() {
        let after = utc("2026-11-01T05:29:59Z");
        let next = next_cron_after("30 1 * * *", "America/New_York", after).unwrap();
        assert_eq!(next, utc("2026-11-01T05:30:00Z"));
        assert!(next > after);
    }

    #[test]
    fn fall_back_advances_past_both_occurrences() {
        let after = utc("2026-11-01T06:30:00Z");
        let next = next_cron_after("30 1 * * *", "America/New_York", after).unwrap();
        assert_eq!(next, utc("2026-11-02T06:30:00Z"));
        assert!(next > after);
    }

    #[test]
    fn spring_forward_gap_moves_to_the_next_real_occurrence() {
        // 2026-03-08 02:30 does not exist in New York.
        let after = utc("2026-03-08T00:00:00Z");
        let next = next_cron_after("30 2 * * *", "America/New_York", after).unwrap();
        assert_eq!(next, utc("2026-03-09T06:30:00Z"));
        assert!(next > after);
    }
}
