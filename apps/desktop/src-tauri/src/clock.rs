//! 版本号（混合逻辑时钟）：每次修改一条要同步的记录，都给它一个新的版本号。
//! 格式 "<毫秒时间戳 13 位>-<计数 4 位>-<设备 ID>"，直接按字符串比较就能分出先后；
//! 几台电脑的时钟差一点、或者同一毫秒里改了好几条，也不会乱序。

use chrono::Utc;
use std::sync::Mutex;

struct Clock {
    ms: i64,
    n: u32,
    device: String,
}

impl Clock {
    fn next(&mut self, now_ms: i64) -> String {
        if now_ms > self.ms {
            self.ms = now_ms;
            self.n = 0;
        } else if self.n >= 9999 {
            // 计数只有 4 位，满了就借用下一毫秒，保证字符串比较仍然正确
            self.ms += 1;
            self.n = 0;
        } else {
            self.n += 1;
        }
        format!("{:013}-{:04}-{}", self.ms, self.n, self.device)
    }

    fn observe(&mut self, ver: &str) {
        let mut it = ver.splitn(3, '-');
        let (Some(ms), Some(n)) = (it.next(), it.next()) else { return };
        let (Ok(ms), Ok(n)) = (ms.parse::<i64>(), n.parse::<u32>()) else { return };
        if (ms, n) > (self.ms, self.n) {
            self.ms = ms;
            self.n = n;
        }
    }
}

static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);

pub fn init(device: &str) {
    *CLOCK.lock().unwrap() = Some(Clock { ms: 0, n: 0, device: device.into() });
}

/// 看到一个已有的版本号（启动时库里最大的那个，以后还有别的设备同步过来的），之后生成的都比它新
pub fn observe(ver: &str) {
    if let Some(c) = CLOCK.lock().unwrap().as_mut() {
        c.observe(ver);
    }
}

pub fn next() -> String {
    let mut g = CLOCK.lock().unwrap();
    let c = g.get_or_insert_with(|| Clock { ms: 0, n: 0, device: "000000000000".into() });
    c.next(Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> Clock {
        Clock { ms: 0, n: 0, device: "dev".into() }
    }

    #[test]
    fn format() {
        assert_eq!(clock().next(1_730_000_000_000), "1730000000000-0000-dev");
    }

    #[test]
    fn same_ms_counts_up() {
        let mut c = clock();
        let a = c.next(1000);
        let b = c.next(1000);
        assert_eq!(b, "0000000001000-0001-dev");
        assert!(b > a);
    }

    #[test]
    fn clock_going_back_stays_ordered() {
        let mut c = clock();
        let a = c.next(5000);
        let b = c.next(4000);
        assert!(b > a);
    }

    #[test]
    fn counter_overflow_borrows_next_ms() {
        let mut c = clock();
        c.next(1000);
        let mut last = String::new();
        for _ in 0..10_000 {
            let v = c.next(1000);
            assert!(v > last);
            last = v;
        }
        assert!(last.starts_with("0000000001001-"));
    }

    #[test]
    fn observe_newer_version() {
        let mut c = clock();
        c.next(1000);
        c.observe("0000000009000-0005-other");
        assert_eq!(c.next(2000), "0000000009000-0006-dev");
        c.observe("0000000001000-0000-other");
        c.observe("garbage");
        assert_eq!(c.next(2000), "0000000009000-0007-dev");
    }
}
