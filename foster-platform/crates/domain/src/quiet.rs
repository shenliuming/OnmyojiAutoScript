use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietWindow {
    pub weekday_mask: u8,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub before_buffer_minutes: i64,
    pub after_buffer_minutes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleGate {
    Open,
    DeferredUntil(DateTime<Utc>),
}

pub fn evaluate_quiet_periods(
    now: DateTime<Utc>,
    timezone: Tz,
    windows: &[QuietWindow],
) -> ScheduleGate {
    let local_now = now.with_timezone(&timezone);
    let local_date = local_now.date_naive();
    let mut latest_end: Option<DateTime<Utc>> = None;

    for offset in -1_i64..=1 {
        let Some(window_date) = local_date.checked_add_signed(Duration::days(offset)) else {
            continue;
        };

        for window in windows {
            if !weekday_enabled(window.weekday_mask, window_date) {
                continue;
            }

            let start = NaiveDateTime::new(window_date, window.start_time);
            let crosses_midnight = window.end_time <= window.start_time;
            let end_date = if crosses_midnight {
                window_date
                    .checked_add_signed(Duration::days(1))
                    .unwrap_or(window_date)
            } else {
                window_date
            };
            let end = NaiveDateTime::new(end_date, window.end_time);

            let protected_start =
                start - Duration::minutes(window.before_buffer_minutes.max(0));
            let protected_end = end + Duration::minutes(window.after_buffer_minutes.max(0));

            let Some(start_utc) = resolve_local(timezone, protected_start, Boundary::Start) else {
                continue;
            };
            let Some(end_utc) = resolve_local(timezone, protected_end, Boundary::End) else {
                continue;
            };

            if now >= start_utc && now < end_utc {
                latest_end = Some(match latest_end {
                    Some(current) => current.max(end_utc),
                    None => end_utc,
                });
            }
        }
    }

    latest_end
        .map(ScheduleGate::DeferredUntil)
        .unwrap_or(ScheduleGate::Open)
}

fn weekday_enabled(mask: u8, date: NaiveDate) -> bool {
    let bit = 1_u8 << date.weekday().num_days_from_monday();
    mask & bit != 0
}

#[derive(Debug, Clone, Copy)]
enum Boundary {
    Start,
    End,
}

fn resolve_local(
    timezone: Tz,
    mut local: NaiveDateTime,
    boundary: Boundary,
) -> Option<DateTime<Utc>> {
    for _ in 0..=180 {
        match timezone.from_local_datetime(&local) {
            LocalResult::Single(value) => return Some(value.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => {
                let selected = match boundary {
                    Boundary::Start => first.min(second),
                    Boundary::End => first.max(second),
                };
                return Some(selected.with_timezone(&Utc));
            }
            LocalResult::None => {
                local += Duration::minutes(1);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn local_utc(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        tz.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    fn monday_only(start_h: u32, start_m: u32, end_h: u32, end_m: u32) -> QuietWindow {
        QuietWindow {
            weekday_mask: 1 << Weekday::Mon.num_days_from_monday(),
            start_time: NaiveTime::from_hms_opt(start_h, start_m, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(end_h, end_m, 0).unwrap(),
            before_buffer_minutes: 0,
            after_buffer_minutes: 0,
        }
    }

    #[test]
    fn outside_window_is_open() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 21, 18, 0);
        let windows = [monday_only(20, 0, 23, 0)];

        assert_eq!(
            evaluate_quiet_periods(now, tz, &windows),
            ScheduleGate::Open
        );
    }

    #[test]
    fn inside_normal_window_defers_to_end() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 21, 21, 0);
        let windows = [monday_only(20, 0, 23, 0)];

        assert_eq!(
            evaluate_quiet_periods(now, tz, &windows),
            ScheduleGate::DeferredUntil(local_utc(tz, 2026, 9, 21, 23, 0))
        );
    }

    #[test]
    fn cross_midnight_window_applies_after_midnight() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 22, 0, 30);
        let windows = [monday_only(23, 0, 1, 0)];

        assert_eq!(
            evaluate_quiet_periods(now, tz, &windows),
            ScheduleGate::DeferredUntil(local_utc(tz, 2026, 9, 22, 1, 0))
        );
    }

    #[test]
    fn weekday_mask_is_respected() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 22, 21, 0);
        let windows = [monday_only(20, 0, 23, 0)];

        assert_eq!(
            evaluate_quiet_periods(now, tz, &windows),
            ScheduleGate::Open
        );
    }

    #[test]
    fn before_buffer_is_protected() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 21, 19, 57);
        let mut window = monday_only(20, 0, 23, 0);
        window.before_buffer_minutes = 5;

        assert_eq!(
            evaluate_quiet_periods(now, tz, &[window]),
            ScheduleGate::DeferredUntil(local_utc(tz, 2026, 9, 21, 23, 0))
        );
    }

    #[test]
    fn after_buffer_extends_defer_time() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 21, 22, 0);
        let mut window = monday_only(20, 0, 23, 0);
        window.after_buffer_minutes = 5;

        assert_eq!(
            evaluate_quiet_periods(now, tz, &[window]),
            ScheduleGate::DeferredUntil(local_utc(tz, 2026, 9, 21, 23, 5))
        );
    }

    #[test]
    fn overlapping_windows_defer_to_latest_end() {
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let now = local_utc(tz, 2026, 9, 21, 21, 30);
        let first = monday_only(20, 0, 22, 0);
        let second = monday_only(21, 0, 23, 30);

        assert_eq!(
            evaluate_quiet_periods(now, tz, &[first, second]),
            ScheduleGate::DeferredUntil(local_utc(tz, 2026, 9, 21, 23, 30))
        );
    }
}
