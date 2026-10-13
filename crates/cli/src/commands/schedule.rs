use fetchx_core_services::CoreServices;
use fetchx_models::Timestamp;

use crate::{
    args::ScheduleOpts,
    commands::parse_id,
    exit::{self, ExitCode},
    output,
};

pub async fn run(svc: &CoreServices, opts: ScheduleOpts, json: bool) -> ExitCode {
    let id = match parse_id(&opts.id) {
        Ok(id) => id,
        Err(_) => {
            output::print_error(json, "invalid download ID format");
            return exit::INVALID_ARGS;
        }
    };

    if opts.clear {
        match svc.scheduler().remove_schedule(id).await {
            Ok(()) => {
                if json {
                    output::print_json_ok(
                        &serde_json::json!({ "ok": true, "action": "schedule_cleared", "id": id.to_string() }),
                    );
                } else {
                    println!("Schedule cleared for {id}");
                }
                return exit::SUCCESS;
            }
            Err(e) => {
                output::print_error(json, &format!("could not clear schedule: {e}"));
                return exit::GENERAL_ERROR;
            }
        }
    }

    let scheduled_at = match parse_datetime(&opts.at) {
        Ok(t) => t,
        Err(msg) => {
            output::print_error(json, &msg);
            return exit::INVALID_ARGS;
        }
    };

    match svc.scheduler().schedule_download(id, scheduled_at).await {
        Ok(()) => {
            if json {
                output::print_json_ok(&serde_json::json!({
                    "ok": true, "action": "scheduled", "id": id.to_string(),
                    "scheduled_at_ms": scheduled_at.as_unix_millis()
                }));
            } else {
                println!("Scheduled {id} to start at {}", opts.at);
            }
            exit::SUCCESS
        }
        Err(e) => {
            output::print_error(json, &format!("could not schedule: {e}"));
            exit::GENERAL_ERROR
        }
    }
}

/// Parse "YYYY-MM-DD HH:MM" or "YYYY-MM-DDTHH:MM" into a Timestamp.
///
/// Does not accept past timestamps.
pub fn parse_datetime(s: &str) -> Result<Timestamp, String> {
    // Normalise to "YYYY-MM-DDTHH:MM" by replacing space with T.
    let normalised = s.trim().replace(' ', "T");
    // Parse individual components.
    let parts: Vec<&str> = normalised.splitn(2, 'T').collect();
    if parts.len() != 2 {
        return Err(format!("expected format YYYY-MM-DD HH:MM, got `{s}`"));
    }
    let date_parts: Vec<&str> = parts[0].splitn(3, '-').collect();
    let time_parts: Vec<&str> = parts[1].splitn(2, ':').collect();
    if date_parts.len() != 3 || time_parts.len() != 2 {
        return Err(format!("expected format YYYY-MM-DD HH:MM, got `{s}`"));
    }

    let year: i32 = date_parts[0]
        .parse()
        .map_err(|_| format!("invalid year in `{s}`"))?;
    let month: u32 = date_parts[1]
        .parse()
        .map_err(|_| format!("invalid month in `{s}`"))?;
    let day: u32 = date_parts[2]
        .parse()
        .map_err(|_| format!("invalid day in `{s}`"))?;
    let hour: u32 = time_parts[0]
        .parse()
        .map_err(|_| format!("invalid hour in `{s}`"))?;
    let min: u32 = time_parts[1]
        .parse()
        .map_err(|_| format!("invalid minute in `{s}`"))?;

    // Validation ranges
    if !(1..=12).contains(&month) {
        return Err(format!("month out of range in `{s}`"));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("day out of range in `{s}`"));
    }
    if hour > 23 {
        return Err(format!("hour out of range in `{s}`"));
    }
    if min > 59 {
        return Err(format!("minute out of range in `{s}`"));
    }

    // Convert to Unix milliseconds using a simple formula (ignores leap seconds,
    // good enough for scheduling). Uses Julian Day Number for date arithmetic.
    let a = (14u32.saturating_sub(month)) / 12;
    let y = year as i64 + 4800 - a as i64;
    let m = month as i64 + 12 * a as i64 - 3;
    let jdn = day as i64 + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045;
    // Unix epoch is JDN 2440588.
    let days_since_epoch = jdn - 2_440_588;
    let seconds = days_since_epoch * 86_400 + hour as i64 * 3600 + min as i64 * 60;
    let millis = seconds * 1000;

    if millis < 0 {
        return Err("timestamp is before the Unix epoch".to_owned());
    }

    let ts = Timestamp::from_unix_millis(millis as u64);
    let now = Timestamp::now().map_err(|e| format!("could not read system time: {e}"))?;
    if ts <= now {
        return Err(format!("schedule time `{s}` is in the past"));
    }

    Ok(ts)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_future_datetime_succeeds() {
        // Far future — always valid.
        let result = parse_datetime("2099-12-31 23:59");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn parse_past_datetime_returns_error() {
        let result = parse_datetime("2000-01-01 00:00");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("in the past"));
    }

    #[test]
    fn parse_invalid_format_returns_error() {
        assert!(parse_datetime("not-a-date").is_err());
        assert!(parse_datetime("2099/12/31 23:59").is_err());
    }

    #[test]
    fn parse_iso_t_separator() {
        let result = parse_datetime("2099-06-15T10:30");
        assert!(result.is_ok(), "{result:?}");
    }
}
