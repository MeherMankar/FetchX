use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub(crate) struct RetryPolicy {
    pub max_attempts: u8,
    pub max_duration: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl RetryPolicy {
    pub(crate) const HTTP: Self = Self {
        max_attempts: 4,
        max_duration: Duration::from_secs(30),
        initial_backoff: Duration::from_millis(250),
        max_backoff: Duration::from_secs(8),
    };

    /// Retry policy tuned for stateful FTP/FTPS connections.
    ///
    /// FTP uses a persistent TCP control connection; a single connect timeout of
    /// 15 s can consume half of an HTTP-tuned 30 s window, leaving at most one
    /// useful retry attempt.  This policy uses a 120 s window and 5 attempts to
    /// give slow or intermittently reachable FTP servers a realistic chance to
    /// recover before the budget is exhausted.
    pub(crate) const FTP: Self = Self {
        max_attempts: 5,
        max_duration: Duration::from_secs(120),
        initial_backoff: Duration::from_millis(500),
        max_backoff: Duration::from_secs(16),
    };

    /// Retry policy for resumable SFTP transfers.
    ///
    /// Each attempt reconnects and revalidates remote metadata and the saved
    /// partial prefix before resuming.
    pub(crate) const SFTP: Self = Self {
        max_attempts: 4,
        max_duration: Duration::from_secs(120),
        initial_backoff: Duration::from_millis(250),
        max_backoff: Duration::from_secs(8),
    };

    pub(crate) fn delay(
        self,
        failed_attempt: u8,
        retry_after: Option<Duration>,
        elapsed: Duration,
    ) -> Option<Duration> {
        if failed_attempt >= self.max_attempts || elapsed >= self.max_duration {
            return None;
        }

        let exponent = u32::from(failed_attempt.saturating_sub(1)).min(16);
        let backoff = self
            .initial_backoff
            .saturating_mul(2_u32.saturating_pow(exponent))
            .min(self.max_backoff);
        let jitter = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_millis()
            % 250;
        let delay = retry_after
            .unwrap_or_else(|| backoff.saturating_add(Duration::from_millis(jitter.into())));
        (elapsed.saturating_add(delay) <= self.max_duration).then_some(delay)
    }

    pub(crate) const fn is_transient_status(status: u16) -> bool {
        status == 408 || status == 425 || status == 429 || (status >= 500 && status <= 599)
    }

    pub(crate) fn is_transient_request(error: &reqwest::Error) -> bool {
        error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
    }
}

pub(crate) fn retry_after_delay(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let (_, date) = value.trim().split_once(", ")?;
    let mut fields = date.split_whitespace();
    let day = fields.next()?.parse::<u32>().ok()?;
    let month = match fields.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = fields.next()?.parse::<i64>().ok()?;
    let time = fields.next()?;
    if fields.next()? != "GMT" || fields.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let hour = clock.next()?.parse::<u64>().ok()?;
    let minute = clock.next()?.parse::<u64>().ok()?;
    let second = clock.next()?.parse::<u64>().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days_in_month = match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days_in_month {
        return None;
    }

    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let unix_days = era * 146_097 + day_of_era - 719_468;
    let timestamp = unix_days
        .checked_mul(86_400)?
        .checked_add(i64::try_from(hour * 3_600 + minute * 60 + second).ok()?)?;
    let now = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    Some(Duration::from_secs(
        u64::try_from(timestamp.saturating_sub(now).max(0)).ok()?,
    ))
}

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RetryPolicy;

    #[test]
    fn retry_delays_are_bounded_and_exponential() {
        let first = RetryPolicy::HTTP
            .delay(1, None, Duration::ZERO)
            .expect("first retry");
        let second = RetryPolicy::HTTP
            .delay(2, None, Duration::ZERO)
            .expect("second retry");
        assert!(first >= Duration::from_millis(250));
        assert!(first < Duration::from_millis(500));
        assert!(second >= Duration::from_millis(500));
        assert!(second < Duration::from_millis(750));
        assert!(RetryPolicy::HTTP.delay(4, None, Duration::ZERO).is_none());
    }

    #[test]
    fn retry_after_and_retry_duration_are_respected() {
        assert_eq!(
            RetryPolicy::HTTP.delay(1, Some(Duration::from_secs(2)), Duration::ZERO),
            Some(Duration::from_secs(2))
        );
        assert!(
            RetryPolicy::HTTP
                .delay(1, Some(Duration::from_secs(31)), Duration::ZERO)
                .is_none()
        );
        assert!(
            RetryPolicy::HTTP
                .delay(1, None, Duration::from_secs(30))
                .is_none()
        );
    }

    #[test]
    fn sftp_retry_budget_is_bounded() {
        assert_eq!(RetryPolicy::SFTP.max_attempts, 4);
        assert_eq!(RetryPolicy::SFTP.max_duration, Duration::from_secs(120));
        let first = RetryPolicy::SFTP
            .delay(1, None, Duration::ZERO)
            .expect("first SFTP retry");
        assert!(first >= Duration::from_millis(250));
        assert!(first < Duration::from_millis(500));
        assert!(RetryPolicy::SFTP.delay(4, None, Duration::ZERO).is_none());
        assert!(
            RetryPolicy::SFTP
                .delay(1, None, Duration::from_secs(120))
                .is_none()
        );
    }

    #[test]
    fn retries_only_transient_http_statuses() {
        for status in [408, 425, 429, 500, 503, 599] {
            assert!(RetryPolicy::is_transient_status(status));
        }
        for status in [200, 400, 401, 403, 404, 499, 600] {
            assert!(!RetryPolicy::is_transient_status(status));
        }
    }

    #[test]
    fn retry_after_accepts_delta_seconds_and_http_dates() {
        assert_eq!(
            super::retry_after_delay("12"),
            Some(Duration::from_secs(12))
        );
        assert!(
            super::retry_after_delay("Thu, 01 Jan 2099 00:00:00 GMT")
                .is_some_and(|delay| delay > Duration::ZERO)
        );
        assert!(super::retry_after_delay("not a date").is_none());
    }
}
