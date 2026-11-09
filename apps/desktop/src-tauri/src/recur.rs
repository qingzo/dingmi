//! 周期提醒：算出下一次该响的时间。

use crate::db::Repeat;
use crate::lunar;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};

fn last_day_of_month(d: NaiveDate) -> u32 {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|n| n.pred_opt())
        .map(|n| n.day())
        .unwrap_or(28)
}

/// 这天是这个月的第几个周几
fn nth_of(d: NaiveDate) -> i32 {
    (d.day() as i32 + 6) / 7
}

/// `base` 是这条提醒第一次的日期，没填的字段（几号、几月、第几个）沿用它。格式见 docs/方案/同步方案.md 4.2
pub fn matches(r: &Repeat, base: NaiveDate, d: NaiveDate) -> bool {
    let wd = d.weekday().number_from_monday();
    match r.kind.as_str() {
        "daily" => true,
        "weekdays" => wd <= 5,
        "weekly" => {
            let day_ok = if r.days.is_empty() {
                wd == base.weekday().number_from_monday()
            } else {
                r.days.contains(&wd)
            };
            // 每几周：从 base 当天或之后第一个对上的那天所在的那一周开始数（周一算一周的开始）
            let n = r.every.max(1) as i64;
            if !day_ok || n == 1 {
                return day_ok;
            }
            let anchor = (0..7)
                .map(|i| base + Duration::days(i))
                .find(|x| r.days.is_empty() || r.days.contains(&x.weekday().number_from_monday()))
                .unwrap_or(base);
            let monday = |x: NaiveDate| x - Duration::days(x.weekday().num_days_from_monday() as i64);
            let weeks = (monday(d) - monday(anchor)).num_days() / 7;
            weeks.rem_euclid(n) == 0
        }
        "monthly" => {
            let want = if r.monthday == 0 { base.day() } else { r.monthday };
            d.day() == want.min(last_day_of_month(d))
        }
        "interval" => {
            let n = r.every.max(1) as i64;
            let diff = (d - base).num_days();
            diff >= 0 && diff % n == 0
        }
        "yearly" => {
            let m = if r.month == 0 { base.month() } else { r.month };
            let md = if r.monthday == 0 { base.day() } else { r.monthday };
            // 2 月 29 号在平年是 28 号
            d.month() == m && d.day() == md.min(last_day_of_month(d))
        }
        "month_last" => d.day() == last_day_of_month(d),
        "month_nth" => {
            let want = r.days.first().copied().unwrap_or_else(|| base.weekday().number_from_monday());
            let nth = if r.nth == 0 { nth_of(base) } else { r.nth };
            wd == want && if nth == -1 { d.day() + 7 > last_day_of_month(d) } else { nth_of(d) == nth }
        }
        "lunar_yearly" => {
            // 闰月不算，只在平常那个月提醒；那个月没有三十就在二十九
            let Some(l) = lunar::to_lunar(d).filter(|l| !l.leap) else { return false };
            let b = if r.month == 0 || r.monthday == 0 { lunar::to_lunar(base) } else { None };
            let m = if r.month == 0 { b.map(|b| b.month).unwrap_or(1) } else { r.month };
            let md = if r.monthday == 0 { b.map(|b| b.day).unwrap_or(1) } else { r.monthday };
            l.month == m && l.day == md.min(lunar::month_days(l.year, l.month, false))
        }
        _ => false,
    }
}

/// 严格晚于 `after` 的下一次时间，沿用 `due` 的钟点。
pub fn next_after(r: &Repeat, due: NaiveDateTime, after: NaiveDateTime) -> Option<NaiveDateTime> {
    if !r.is_recurring() {
        return None;
    }
    let time = due.time();
    let base = due.date();
    let mut d = after.date();
    for _ in 0..800 {
        let cand = d.and_time(time);
        if cand > after && matches(r, base, d) {
            return Some(cand);
        }
        d = d.succ_opt()?;
    }
    None
}

/// 新建或修改周期提醒时，把开始时间对齐到第一个符合规则、且不早于现在的时间。
pub fn normalize(r: &Repeat, due: NaiveDateTime, now: NaiveDateTime) -> NaiveDateTime {
    if !r.is_recurring() {
        return due;
    }
    let after = if due > now { due - Duration::minutes(1) } else { now };
    next_after(r, due, after).unwrap_or(due)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn rep(kind: &str) -> Repeat {
        Repeat { kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn daily_rolls_to_tomorrow() {
        let n = next_after(&rep("daily"), dt("2026-10-31 08:30"), dt("2026-10-31 15:00")).unwrap();
        assert_eq!(n, dt("2026-11-01 08:30"));
    }

    #[test]
    fn weekdays_skip_weekend() {
        // 2026-10-30 is a Friday
        let n = next_after(&rep("weekdays"), dt("2026-10-30 17:50"), dt("2026-10-30 18:00")).unwrap();
        assert_eq!(n, dt("2026-11-02 17:50"));
    }

    #[test]
    fn weekly_days() {
        let r = Repeat { kind: "weekly".into(), days: vec![2, 4, 6], ..Default::default() };
        // Sat 2026-10-31 after 19:30 -> Tue 2026-11-03
        let n = next_after(&r, dt("2026-10-31 19:30"), dt("2026-10-31 19:31")).unwrap();
        assert_eq!(n, dt("2026-11-03 19:30"));
    }

    #[test]
    fn monthly_clamps_to_month_end() {
        let r = Repeat { kind: "monthly".into(), monthday: 31, ..Default::default() };
        let n = next_after(&r, dt("2026-10-31 09:00"), dt("2026-10-31 10:00")).unwrap();
        assert_eq!(n, dt("2026-11-30 09:00"));
    }

    #[test]
    fn interval_days() {
        let r = Repeat { kind: "interval".into(), every: 5, ..Default::default() };
        let n = next_after(&r, dt("2026-10-31 10:00"), dt("2026-10-31 10:00")).unwrap();
        assert_eq!(n, dt("2026-11-05 10:00"));
    }

    #[test]
    fn normalize_future_keeps_matching_due() {
        let r = Repeat { kind: "weekly".into(), days: vec![1], ..Default::default() };
        // 2026-11-02 is Monday
        let n = normalize(&r, dt("2026-11-02 09:00"), dt("2026-10-31 12:00"));
        assert_eq!(n, dt("2026-11-02 09:00"));
        // past due rolls forward
        let n = normalize(&rep("daily"), dt("2026-10-31 08:00"), dt("2026-10-31 12:00"));
        assert_eq!(n, dt("2026-11-01 08:00"));
    }

    fn next(r: &Repeat, due: &str, after: &str) -> String {
        next_after(r, dt(due), dt(after)).unwrap().format("%Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn yearly_and_feb29() {
        let r = Repeat { kind: "yearly".into(), month: 3, monthday: 5, ..Default::default() };
        assert_eq!(next(&r, "2026-03-05 09:00", "2026-03-05 10:00"), "2027-03-05 09:00");
        // 2 月 29 号：平年 28 号，闰年 29 号
        let r = Repeat { kind: "yearly".into(), month: 2, monthday: 29, ..Default::default() };
        assert_eq!(next(&r, "2024-02-29 08:00", "2024-03-01 00:00"), "2025-02-28 08:00");
        assert_eq!(next(&r, "2027-02-28 08:00", "2027-03-01 00:00"), "2028-02-29 08:00");
        // 没填月日：沿用开始日期
        assert_eq!(next(&rep("yearly"), "2026-11-05 09:00", "2026-11-05 10:00"), "2027-11-05 09:00");
    }

    #[test]
    fn every_two_weeks() {
        // 2026-11-06 是周五，从这周开始数
        let r = Repeat { kind: "weekly".into(), days: vec![5], every: 2, ..Default::default() };
        assert_eq!(normalize(&r, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-11-06 09:00"));
        assert_eq!(next(&r, "2026-11-06 09:00", "2026-11-06 09:30"), "2026-11-20 09:00");
        assert_eq!(next(&r, "2026-11-20 09:00", "2026-11-20 09:30"), "2026-12-04 09:00");
        // 从周四开始的隔周周一：第一次是最近的那个周一
        assert_eq!(normalize(&Repeat { kind: "weekly".into(), days: vec![1], every: 2, ..Default::default() }, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-11-09 09:00"));
        // 一周两天
        let r = Repeat { kind: "weekly".into(), days: vec![1, 3], every: 2, ..Default::default() };
        assert_eq!(next(&r, "2026-11-02 09:00", "2026-11-02 09:30"), "2026-11-04 09:00");
        assert_eq!(next(&r, "2026-11-04 09:00", "2026-11-04 09:30"), "2026-11-16 09:00");
        // every 是 0 或 1：每周
        let r = Repeat { kind: "weekly".into(), days: vec![5], every: 1, ..Default::default() };
        assert_eq!(next(&r, "2026-11-06 09:00", "2026-11-06 09:30"), "2026-11-13 09:00");
    }

    #[test]
    fn month_last_day() {
        let r = rep("month_last");
        assert_eq!(normalize(&r, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-11-30 09:00"));
        assert_eq!(next(&r, "2026-11-30 09:00", "2026-11-30 10:00"), "2026-12-31 09:00");
        assert_eq!(next(&r, "2027-01-31 09:00", "2027-01-31 10:00"), "2027-02-28 09:00");
        assert_eq!(next(&r, "2028-01-31 09:00", "2028-01-31 10:00"), "2028-02-29 09:00");
    }

    #[test]
    fn month_nth_weekday() {
        // 每月第一个周一
        let r = Repeat { kind: "month_nth".into(), nth: 1, days: vec![1], ..Default::default() };
        assert_eq!(normalize(&r, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-12-07 09:00"));
        assert_eq!(next(&r, "2026-12-07 09:00", "2026-12-07 10:00"), "2027-01-04 09:00");
        // 每月最后一个周五
        let r = Repeat { kind: "month_nth".into(), nth: -1, days: vec![5], ..Default::default() };
        assert_eq!(normalize(&r, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-11-27 09:00"));
        assert_eq!(next(&r, "2026-11-27 09:00", "2026-11-27 10:00"), "2026-12-25 09:00");
        // 第五个周四：没有的月份跳过（2026 年 12 月有 5 个周四：31 号）
        let r = Repeat { kind: "month_nth".into(), nth: 5, days: vec![4], ..Default::default() };
        assert_eq!(normalize(&r, dt("2026-11-05 09:00"), dt("2026-11-05 10:00")), dt("2026-12-31 09:00"));
        // 没填：沿用开始日期（2026-11-05 是第一个周四）
        assert_eq!(next(&rep("month_nth"), "2026-11-05 09:00", "2026-11-05 10:00"), "2026-12-03 09:00");
    }

    #[test]
    fn lunar_yearly() {
        // 农历八月十五：2026-09-25、2027-09-15
        let r = Repeat { kind: "lunar_yearly".into(), month: 8, monthday: 15, ..Default::default() };
        assert_eq!(normalize(&r, dt("2026-08-01 20:00"), dt("2026-08-01 10:00")), dt("2026-09-25 20:00"));
        assert_eq!(next(&r, "2026-09-25 20:00", "2026-09-25 21:00"), "2027-09-15 20:00");
        // 闰月不再响：2025 年闰六月初一（07-25）不算，六月初一是 06-25
        let r = Repeat { kind: "lunar_yearly".into(), month: 6, monthday: 1, ..Default::default() };
        assert_eq!(next(&r, "2025-06-25 09:00", "2025-06-25 10:00"), "2026-07-14 09:00");
        // 腊月三十：2025 年腊月只有二十九（2026-02-16），2023 年腊月有三十（2024-02-09）
        let r = Repeat { kind: "lunar_yearly".into(), month: 12, monthday: 30, ..Default::default() };
        assert_eq!(next(&r, "2024-02-09 09:00", "2024-02-09 10:00"), "2025-01-28 09:00");
        assert_eq!(next(&r, "2025-01-28 09:00", "2025-01-28 10:00"), "2026-02-16 09:00");
        // 没填月日：沿用开始日期的农历（2026-11-05 是九月廿七）
        assert_eq!(next(&rep("lunar_yearly"), "2026-11-05 09:00", "2026-11-05 10:00"), "2027-10-26 09:00");
    }

    #[test]
    fn old_record_without_new_fields() {
        let r: Repeat = serde_json::from_str(r#"{"kind":"weekly","days":[5],"monthday":0,"every":0}"#).unwrap();
        assert_eq!((r.month, r.nth), (0, 0));
        assert_eq!(next(&r, "2026-11-06 09:00", "2026-11-06 10:00"), "2026-11-13 09:00");
    }
}
