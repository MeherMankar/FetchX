use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_ACTIVE_DOWNLOADS: usize = 32;
const MAX_GLOBAL_CONNECTIONS: usize = 64;
const MAX_PER_DOWNLOAD_CONNECTIONS: usize = 32;
const MAX_BANDWIDTH_BYTES_PER_SECOND: u64 = 1_000_000_000_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProfile {
    Unlimited,
    #[default]
    Normal,
    Limited,
    Night,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePressure {
    #[default]
    Normal,
    Elevated,
    High,
    Critical,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct EngineBandwidthLimits {
    pub direct_http_bytes_per_second: Option<u64>,
    pub hls_bytes_per_second: Option<u64>,
    pub aria2_bytes_per_second: Option<u64>,
    pub yt_dlp_bytes_per_second: Option<u64>,
    pub native_torrent_bytes_per_second: Option<u64>,
    /// Shared bandwidth limit for FTP and FTPS engines.
    pub ftp_bytes_per_second: Option<u64>,
    pub sftp_bytes_per_second: Option<u64>,
    pub metalink_bytes_per_second: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ResourcePolicy {
    pub profile: ResourceProfile,
    pub max_active_downloads: usize,
    pub global_connection_limit: usize,
    pub per_download_connection_limit: usize,
    pub global_bandwidth_bytes_per_second: Option<u64>,
    pub per_download_bandwidth_bytes_per_second: Option<u64>,
    pub engine_bandwidth_limits: EngineBandwidthLimits,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self::for_profile(ResourceProfile::Normal)
    }
}

impl ResourcePolicy {
    pub fn engine_bandwidth_limit(&self, engine: crate::EngineKind) -> Option<u64> {
        match engine {
            crate::EngineKind::DirectHttp => {
                self.engine_bandwidth_limits.direct_http_bytes_per_second
            }
            crate::EngineKind::Hls => self.engine_bandwidth_limits.hls_bytes_per_second,
            crate::EngineKind::Aria2 => None,
            crate::EngineKind::YtDlp => self.engine_bandwidth_limits.yt_dlp_bytes_per_second,
            crate::EngineKind::NativeTorrent => {
                self.engine_bandwidth_limits.native_torrent_bytes_per_second
            }
            // FTP and FTPS share the same per-engine bandwidth limit.
            crate::EngineKind::Ftp | crate::EngineKind::Ftps => {
                self.engine_bandwidth_limits.ftp_bytes_per_second
            }
            crate::EngineKind::Sftp => self.engine_bandwidth_limits.sftp_bytes_per_second,
            crate::EngineKind::Metalink => self.engine_bandwidth_limits.metalink_bytes_per_second,
        }
    }

    pub fn for_profile(profile: ResourceProfile) -> Self {
        match profile {
            ResourceProfile::Unlimited => Self {
                profile,
                max_active_downloads: 8,
                global_connection_limit: MAX_GLOBAL_CONNECTIONS,
                per_download_connection_limit: MAX_PER_DOWNLOAD_CONNECTIONS,
                global_bandwidth_bytes_per_second: None,
                per_download_bandwidth_bytes_per_second: None,
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
            },
            ResourceProfile::Normal => Self {
                profile,
                max_active_downloads: 3,
                global_connection_limit: 16,
                per_download_connection_limit: 8,
                global_bandwidth_bytes_per_second: None,
                per_download_bandwidth_bytes_per_second: None,
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
            },
            ResourceProfile::Limited => Self {
                profile,
                max_active_downloads: 2,
                global_connection_limit: 8,
                per_download_connection_limit: 4,
                global_bandwidth_bytes_per_second: Some(2 * 1024 * 1024),
                per_download_bandwidth_bytes_per_second: Some(1024 * 1024),
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
            },
            ResourceProfile::Night => Self {
                profile,
                max_active_downloads: 2,
                global_connection_limit: 4,
                per_download_connection_limit: 2,
                global_bandwidth_bytes_per_second: Some(512 * 1024),
                per_download_bandwidth_bytes_per_second: Some(256 * 1024),
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
            },
            ResourceProfile::Custom => Self {
                profile,
                ..Self::default()
            },
        }
    }

    pub fn validate(&self) -> Result<(), ResourcePolicyError> {
        if self.max_active_downloads == 0 || self.max_active_downloads > MAX_ACTIVE_DOWNLOADS {
            return Err(ResourcePolicyError::ActiveDownloadLimit);
        }
        if self.global_connection_limit == 0
            || self.global_connection_limit > MAX_GLOBAL_CONNECTIONS
        {
            return Err(ResourcePolicyError::GlobalConnectionLimit);
        }
        if self.per_download_connection_limit == 0
            || self.per_download_connection_limit > MAX_PER_DOWNLOAD_CONNECTIONS
            || self.per_download_connection_limit > self.global_connection_limit
        {
            return Err(ResourcePolicyError::PerDownloadConnectionLimit);
        }
        for rate in [
            self.global_bandwidth_bytes_per_second,
            self.per_download_bandwidth_bytes_per_second,
            self.engine_bandwidth_limits.direct_http_bytes_per_second,
            self.engine_bandwidth_limits.hls_bytes_per_second,
            self.engine_bandwidth_limits.yt_dlp_bytes_per_second,
            self.engine_bandwidth_limits.native_torrent_bytes_per_second,
            self.engine_bandwidth_limits.ftp_bytes_per_second,
            self.engine_bandwidth_limits.sftp_bytes_per_second,
            self.engine_bandwidth_limits.metalink_bytes_per_second,
        ]
        .into_iter()
        .flatten()
        {
            if rate == 0 || rate > MAX_BANDWIDTH_BYTES_PER_SECOND {
                return Err(ResourcePolicyError::BandwidthLimit);
            }
        }
        if self
            .global_bandwidth_bytes_per_second
            .is_some_and(|rate| rate < self.max_active_downloads as u64)
        {
            return Err(ResourcePolicyError::BandwidthLimit);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WeeklyQueueSchedule {
    pub enabled: bool,
    /// Monday through Sunday.
    pub weekdays: [bool; 7],
    /// Minutes after midnight in the system's local time zone.
    pub start_minute: u16,
    /// Minutes after midnight in the system's local time zone.
    pub end_minute: u16,
}

impl Default for WeeklyQueueSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            weekdays: [true; 7],
            start_minute: 22 * 60,
            end_minute: 6 * 60,
        }
    }
}

impl WeeklyQueueSchedule {
    pub fn validate(&self) -> Result<(), WeeklyQueueScheduleError> {
        if self.start_minute >= 24 * 60 || self.end_minute >= 24 * 60 {
            return Err(WeeklyQueueScheduleError::InvalidTime);
        }
        if self.start_minute == self.end_minute {
            return Err(WeeklyQueueScheduleError::EmptyWindow);
        }
        if self.enabled && !self.weekdays.iter().any(|selected| *selected) {
            return Err(WeeklyQueueScheduleError::NoDaysSelected);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_open_at(&self, weekday: usize, minute: u16) -> bool {
        if weekday >= self.weekdays.len() || minute >= 24 * 60 {
            return false;
        }
        if self.start_minute < self.end_minute {
            return self.weekdays[weekday]
                && minute >= self.start_minute
                && minute < self.end_minute;
        }
        let previous_weekday = (weekday + self.weekdays.len() - 1) % self.weekdays.len();
        (self.weekdays[weekday] && minute >= self.start_minute)
            || (self.weekdays[previous_weekday] && minute < self.end_minute)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum WeeklyQueueScheduleError {
    #[error("weekly queue schedule times must be within a day")]
    InvalidTime,
    #[error("weekly queue schedule start and end times must differ")]
    EmptyWindow,
    #[error("select at least one weekday for an enabled weekly queue schedule")]
    NoDaysSelected,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ResourcePolicyError {
    #[error("active download limit must be between 1 and 32")]
    ActiveDownloadLimit,
    #[error("global connection limit must be between 1 and 64")]
    GlobalConnectionLimit,
    #[error("per-download connection limit must be between 1 and the global limit, up to 32")]
    PerDownloadConnectionLimit,
    #[error("bandwidth limits must be between 1 byte/s and 1 TB/s")]
    BandwidthLimit,
}

#[cfg(test)]
mod tests {
    use super::{
        ResourcePolicy, ResourcePolicyError, ResourceProfile, WeeklyQueueSchedule,
        WeeklyQueueScheduleError,
    };

    #[test]
    fn profiles_have_bounded_documented_defaults() {
        for profile in [
            ResourceProfile::Unlimited,
            ResourceProfile::Normal,
            ResourceProfile::Limited,
            ResourceProfile::Night,
        ] {
            assert!(ResourcePolicy::for_profile(profile).validate().is_ok());
        }
        let normal = ResourcePolicy::default();
        assert_eq!(normal.profile, ResourceProfile::Normal);
        assert_eq!(normal.max_active_downloads, 3);
        assert_eq!(normal.global_connection_limit, 16);
        assert_eq!(normal.per_download_connection_limit, 8);
        let unlimited = ResourcePolicy::for_profile(ResourceProfile::Unlimited);
        assert_eq!(unlimited.max_active_downloads, 8);
        assert_eq!(unlimited.global_connection_limit, 64);
        assert_eq!(unlimited.per_download_connection_limit, 32);
        assert_eq!(unlimited.global_bandwidth_bytes_per_second, None);
        let limited = ResourcePolicy::for_profile(ResourceProfile::Limited);
        assert_eq!(limited.max_active_downloads, 2);
        assert_eq!(limited.global_connection_limit, 8);
        assert_eq!(limited.per_download_connection_limit, 4);
        assert_eq!(
            limited.global_bandwidth_bytes_per_second,
            Some(2 * 1024 * 1024)
        );
        assert_eq!(
            limited.per_download_bandwidth_bytes_per_second,
            Some(1024 * 1024)
        );
        let night = ResourcePolicy::for_profile(ResourceProfile::Night);
        assert_eq!(night.max_active_downloads, 2);
        assert_eq!(night.global_connection_limit, 4);
        assert_eq!(night.per_download_connection_limit, 2);
        assert_eq!(night.global_bandwidth_bytes_per_second, Some(512 * 1024));
        assert_eq!(
            night.per_download_bandwidth_bytes_per_second,
            Some(256 * 1024)
        );
    }

    #[test]
    fn rejects_out_of_bounds_connections_and_bandwidth() {
        let policy = ResourcePolicy {
            per_download_connection_limit: 17,
            ..ResourcePolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(ResourcePolicyError::PerDownloadConnectionLimit)
        );

        let policy = ResourcePolicy {
            global_bandwidth_bytes_per_second: Some(0),
            ..ResourcePolicy::default()
        };
        assert_eq!(policy.validate(), Err(ResourcePolicyError::BandwidthLimit));

        let policy = ResourcePolicy {
            max_active_downloads: 4,
            global_bandwidth_bytes_per_second: Some(3),
            ..ResourcePolicy::default()
        };
        assert_eq!(policy.validate(), Err(ResourcePolicyError::BandwidthLimit));
    }

    #[test]
    fn old_serialized_policy_fields_receive_safe_defaults() -> Result<(), serde_json::Error> {
        let policy: ResourcePolicy = serde_json::from_str(r#"{"profile":"normal"}"#)?;
        assert!(policy.validate().is_ok());
        assert_eq!(policy, ResourcePolicy::default());
        // New bandwidth fields must default to None when deserializing old payloads.
        assert_eq!(policy.engine_bandwidth_limits.ftp_bytes_per_second, None);
        assert_eq!(policy.engine_bandwidth_limits.sftp_bytes_per_second, None);
        assert_eq!(
            policy.engine_bandwidth_limits.metalink_bytes_per_second,
            None
        );
        Ok(())
    }

    #[test]
    fn obsolete_ffmpeg_policy_fields_are_ignored() -> Result<(), serde_json::Error> {
        let policy: ResourcePolicy = serde_json::from_str(
            r#"{"profile":"normal","max_ffmpeg_processes":2,"engine_bandwidth_limits":{"ffmpeg_bytes_per_second":1024}}"#,
        )?;
        assert_eq!(policy, ResourcePolicy::default());
        Ok(())
    }

    #[test]
    fn rejects_integer_overflow_and_values_above_policy_ceilings() {
        assert!(serde_json::from_str::<ResourcePolicy>(r#"{"max_active_downloads":-1}"#).is_err());
        assert!(
            serde_json::from_str::<ResourcePolicy>(
                r#"{"max_active_downloads":18446744073709551616}"#
            )
            .is_err()
        );
        let policy = ResourcePolicy {
            global_connection_limit: 65,
            ..ResourcePolicy::default()
        };
        assert_eq!(
            policy.validate(),
            Err(ResourcePolicyError::GlobalConnectionLimit)
        );
    }

    #[test]
    fn weekly_queue_schedule_handles_selected_days_and_overnight_windows() {
        let schedule = WeeklyQueueSchedule {
            enabled: true,
            weekdays: [true, false, false, false, false, false, false],
            start_minute: 22 * 60,
            end_minute: 6 * 60,
        };

        assert!(schedule.is_open_at(0, 22 * 60));
        assert!(schedule.is_open_at(1, 5 * 60));
        assert!(!schedule.is_open_at(1, 6 * 60));
        assert!(!schedule.is_open_at(1, 22 * 60));
    }

    #[test]
    fn weekly_queue_schedule_validates_enabled_days_and_window_bounds() {
        let mut schedule = WeeklyQueueSchedule {
            enabled: true,
            weekdays: [false; 7],
            ..WeeklyQueueSchedule::default()
        };
        assert_eq!(
            schedule.validate(),
            Err(WeeklyQueueScheduleError::NoDaysSelected)
        );

        schedule.weekdays[0] = true;
        schedule.end_minute = schedule.start_minute;
        assert_eq!(
            schedule.validate(),
            Err(WeeklyQueueScheduleError::EmptyWindow)
        );

        schedule.end_minute = 30;
        schedule.start_minute = 24 * 60;
        assert_eq!(
            schedule.validate(),
            Err(WeeklyQueueScheduleError::InvalidTime)
        );
    }
}
