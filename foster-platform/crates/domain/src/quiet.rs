use chrono::{DateTime, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

#[cfg(test)]
mod tests {
    use super::*;

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
