//! 农历：1900–2100 年。和手机网页、界面用的 shared/lunar.ts 是同一份表，两边用 shared/lunar.cases.json 核对。
//!
//! 每年一个数：低 4 位是闰几月（0 没有闰月）；0x8000 … 0x10 依次是正月 … 腊月，1 是大月（30 天）、0 是小月（29 天）；
//! 0x10000 是闰月是不是大月。1900 年正月初一是公历 1900-01-31。

use chrono::{Duration, NaiveDate};

const INFO: [u32; 201] = [
    0x04bd8, 0x04ae0, 0x0a570, 0x054d5, 0x0d260, 0x0d950, 0x16554, 0x056a0, 0x09ad0, 0x055d2, // 1900
    0x04ae0, 0x0a5b6, 0x0a4d0, 0x0d250, 0x1d255, 0x0b540, 0x0d6a0, 0x0ada2, 0x095b0, 0x14977, // 1910
    0x04970, 0x0a4b0, 0x0b4b5, 0x06a50, 0x06d40, 0x1ab54, 0x02b60, 0x09570, 0x052f2, 0x04970, // 1920
    0x06566, 0x0d4a0, 0x0ea50, 0x16a95, 0x05ad0, 0x02b60, 0x186e3, 0x092e0, 0x1c8d7, 0x0c950, // 1930
    0x0d4a0, 0x1d8a6, 0x0b550, 0x056a0, 0x1a5b4, 0x025d0, 0x092d0, 0x0d2b2, 0x0a950, 0x0b557, // 1940
    0x06ca0, 0x0b550, 0x15355, 0x04da0, 0x0a5b0, 0x14573, 0x052b0, 0x0a9a8, 0x0e950, 0x06aa0, // 1950
    0x0aea6, 0x0ab50, 0x04b60, 0x0aae4, 0x0a570, 0x05260, 0x0f263, 0x0d950, 0x05b57, 0x056a0, // 1960
    0x096d0, 0x04dd5, 0x04ad0, 0x0a4d0, 0x0d4d4, 0x0d250, 0x0d558, 0x0b540, 0x0b6a0, 0x195a6, // 1970
    0x095b0, 0x049b0, 0x0a974, 0x0a4b0, 0x0b27a, 0x06a50, 0x06d40, 0x0af46, 0x0ab60, 0x09570, // 1980
    0x04af5, 0x04970, 0x064b0, 0x074a3, 0x0ea50, 0x06b58, 0x05ac0, 0x0ab60, 0x096d5, 0x092e0, // 1990
    0x0c960, 0x0d954, 0x0d4a0, 0x0da50, 0x07552, 0x056a0, 0x0abb7, 0x025d0, 0x092d0, 0x0cab5, // 2000
    0x0a950, 0x0b4a0, 0x0baa4, 0x0ad50, 0x055d9, 0x04ba0, 0x0a5b0, 0x15176, 0x052b0, 0x0a930, // 2010
    0x07954, 0x06aa0, 0x0ad50, 0x05b52, 0x04b60, 0x0a6e6, 0x0a4e0, 0x0d260, 0x0ea65, 0x0d530, // 2020
    0x05aa0, 0x076a3, 0x096d0, 0x04afb, 0x04ad0, 0x0a4d0, 0x1d0b6, 0x0d250, 0x0d520, 0x0dd45, // 2030
    0x0b5a0, 0x056d0, 0x055b2, 0x049b0, 0x0a577, 0x0a4b0, 0x0aa50, 0x1b255, 0x06d20, 0x0ada0, // 2040
    0x14b63, 0x09370, 0x049f8, 0x04970, 0x064b0, 0x168a6, 0x0ea50, 0x06b20, 0x1a6c4, 0x0aae0, // 2050
    0x092e0, 0x0d2e3, 0x0c960, 0x0d557, 0x0d4a0, 0x0da50, 0x05d55, 0x056a0, 0x0a6d0, 0x055d4, // 2060
    0x052d0, 0x0a9b8, 0x0a950, 0x0b4a0, 0x0b6a6, 0x0ad50, 0x055a0, 0x0aba4, 0x0a5b0, 0x052b0, // 2070
    0x0b273, 0x06930, 0x07337, 0x06aa0, 0x0ad50, 0x14b55, 0x04b60, 0x0a570, 0x054e4, 0x0d160, // 2080
    0x0e968, 0x0d520, 0x0daa0, 0x16aa6, 0x056d0, 0x04ae0, 0x0a9d4, 0x0a2d0, 0x0d150, 0x0f252, // 2090
    0x0d520, // 2100
];
const FIRST: i32 = 1900;
const LAST: i32 = 2100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lunar {
    pub year: i32,
    /// 1–12；闰月也写它跟着的那个月
    pub month: u32,
    pub day: u32,
    pub leap: bool,
}

fn info(y: i32) -> u32 {
    if (FIRST..=LAST).contains(&y) {
        INFO[(y - FIRST) as usize]
    } else {
        0
    }
}

fn start() -> NaiveDate {
    NaiveDate::from_ymd_opt(1900, 1, 31).unwrap()
}

/// 这个月有几天；没有这个月（不是闰月却问闰月、超出范围）是 0
pub fn month_days(y: i32, m: u32, leap: bool) -> u32 {
    let v = info(y);
    if v == 0 || !(1..=12).contains(&m) {
        return 0;
    }
    if leap {
        return if v & 0xf == m { if v & 0x10000 != 0 { 30 } else { 29 } } else { 0 };
    }
    if v & (0x10000 >> m) != 0 {
        30
    } else {
        29
    }
}

fn year_days(y: i32) -> i64 {
    (1..=12).map(|m| (month_days(y, m, false) + month_days(y, m, true)) as i64).sum()
}

/// 公历 → 农历；超出 1900–2100 返回 None
pub fn to_lunar(d: NaiveDate) -> Option<Lunar> {
    let mut off = (d - start()).num_days();
    if off < 0 {
        return None;
    }
    for y in FIRST..=LAST {
        let yd = year_days(y);
        if off >= yd {
            off -= yd;
            continue;
        }
        for m in 1..=12 {
            for leap in [false, true] {
                let md = month_days(y, m, leap) as i64;
                if off < md {
                    return Some(Lunar { year: y, month: m, day: off as u32 + 1, leap });
                }
                off -= md;
            }
        }
    }
    None
}

/// 农历 → 公历；没有这一天返回 None
#[cfg_attr(not(test), allow(dead_code))]
pub fn from_lunar(y: i32, m: u32, d: u32, leap: bool) -> Option<NaiveDate> {
    if d < 1 || d > month_days(y, m, leap) {
        return None;
    }
    let mut off: i64 = (FIRST..y).map(year_days).sum();
    off += (1..m).map(|i| (month_days(y, i, false) + month_days(y, i, true)) as i64).sum::<i64>();
    if leap {
        off += month_days(y, m, false) as i64;
    }
    Some(start() + Duration::days(off + d as i64 - 1))
}

const MONTHS: [&str; 12] = ["正月", "二月", "三月", "四月", "五月", "六月", "七月", "八月", "九月", "十月", "冬月", "腊月"];

/// "八月十五"、"正月初三"
pub fn label(m: u32, d: u32) -> String {
    const ONE: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    let day = match d {
        10 => "初十".to_string(),
        20 => "二十".to_string(),
        30 => "三十".to_string(),
        _ => format!("{}{}", ["初", "十", "廿", "三"][(d / 10).min(3) as usize], ONE[((d.max(1) - 1) % 10) as usize]),
    };
    format!("{}{day}", MONTHS.get((m.max(1) - 1) as usize).copied().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn same_cases_as_ts() {
        let v: Value = serde_json::from_str(include_str!("../../../../shared/lunar.cases.json")).unwrap();
        let cases = v["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        for c in cases {
            let d = NaiveDate::parse_from_str(c[0].as_str().unwrap(), "%Y-%m-%d").unwrap();
            let want = Lunar {
                year: c[1].as_i64().unwrap() as i32,
                month: c[2].as_u64().unwrap() as u32,
                day: c[3].as_u64().unwrap() as u32,
                leap: c[4].as_bool().unwrap(),
            };
            assert_eq!(to_lunar(d), Some(want), "{d}");
            assert_eq!(from_lunar(want.year, want.month, want.day, want.leap), Some(d), "{d}");
        }
    }

    #[test]
    fn edges() {
        assert_eq!(to_lunar(NaiveDate::from_ymd_opt(1900, 1, 30).unwrap()), None);
        // 2025 年没有闰五月，有闰六月；腊月是小月
        assert_eq!(from_lunar(2025, 5, 1, true), None);
        assert_eq!(month_days(2025, 6, true), 29);
        assert_eq!(from_lunar(2025, 12, 30, false), None);
        assert_eq!(label(8, 15), "八月十五");
        assert_eq!(label(1, 3), "正月初三");
        assert_eq!(label(12, 29), "腊月廿九");
        assert_eq!(label(11, 20), "冬月二十");
    }
}
