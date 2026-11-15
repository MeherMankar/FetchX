use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::post_queue_action::PostQueueAction;
use crate::proxy::DesktopProxyConfig;

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
    #[serde(default)]
    pub weekly_bandwidth_schedule: WeeklyBandwidthSchedule,
    #[serde(default)]
    pub post_queue_action: PostQueueAction,
    #[serde(default)]
    pub desktop_proxy: Option<DesktopProxyConfig>,
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
                weekly_bandwidth_schedule: WeeklyBandwidthSchedule::default(),
                post_queue_action: PostQueueAction::default(),
                desktop_proxy: None,
            },
            ResourceProfile::Normal => Self {
                profile,
                max_active_downloads: 3,
                global_connection_limit: 16,
                per_download_connection_limit: 8,
                global_bandwidth_bytes_per_second: None,
                per_download_bandwidth_bytes_per_second: None,
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
                weekly_bandwidth_schedule: WeeklyBandwidthSchedule::default(),
                post_queue_action: PostQueueAction::default(),
                desktop_proxy: None,
            },
            ResourceProfile::Limited => Self {
                profile,
                max_active_downloads: 2,
                global_connection_limit: 8,
                per_download_connection_limit: 4,
                global_bandwidth_bytes_per_second: Some(2 * 1024 * 1024),
                per_download_bandwidth_bytes_per_second: Some(1024 * 1024),
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
                weekly_bandwidth_schedule: WeeklyBandwidthSchedule::default(),
                post_queue_action: PostQueueAction::default(),
                desktop_proxy: None,
            },
            ResourceProfile::Night => Self {
                profile,
                max_active_downloads: 2,
                global_connection_limit: 4,
                per_download_connection_limit: 2,
                global_bandwidth_bytes_per_second: Some(512 * 1024),
                per_download_bandwidth_bytes_per_second: Some(256 * 1024),
                engine_bandwidth_limits: EngineBandwidthLimits::default(),
                weekly_bandwidth_schedule: WeeklyBandwidthSchedule::default(),
                post_queue_action: PostQueueAction::default(),
                desktop_proxy: None,
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
        for window in &self.weekly_bandwidth_schedule.windows {
            if window.limit_bps == 0 || window.limit_bps > MAX_BANDWIDTH_BYTES_PER_SECOND {
                return Err(ResourcePolicyError::BandwidthLimit);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct BandwidthWindow {
    pub weekdays: [bool; 7],
    pub start_minute: u16,
    pub end_minute: u16,
    pub limit_bps: u64,
}

impl BandwidthWindow {
    pub fn is_active_at(&self, weekday: usize, minute: u16) -> bool {
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WeeklyBandwidthSchedule {
    pub windows: Vec<BandwidthWindow>,
}

impl WeeklyBandwidthSchedule {
    pub fn active_limit_bps(&self, weekday: usize, minute: u16) -> Option<u64> {
        self.windows
            .iter()
            .filter(|w| w.is_active_at(weekday, minute))
            .map(|w| w.limit_bps)
            .min()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WeeklyQueueSchedule {
    pub enabled: bool,
    pub weekdays: [bool; 7],
    pub start_minute: u16,
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
        BandwidthWindow, ResourcePolicy, ResourcePolicyError, ResourceProfile, WeeklyQueueSchedule,
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
            let policy = ResourcePolicy::for_profile(profile);
            assert!(policy.validate().is_ok(), "profile {profile:?} should validate");
        }
    }

    #[test]
    fn custom_profile_defaults_are_valid() {
        let policy = ResourcePolicy::for_profile(ResourceProfile::Custom);
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn bandwidth_window_same_day() {
        let w = BandwidthWindow {
            weekdays: [true; 7],
            start_minute: 9 * 60,
            end_minute: 17 * 60,
            limit_bps: 1_000_000,
        };
        assert!(w.is_active_at(0, 9 * 60));
        assert!(w.is_active_at(3, 12 * 60));
        assert!(!w.is_active_at(0, 8 * 60));
        assert!(!w.is_active_at(0, 17 * 60));
    }

    #[test]
    fn bandwidth_window_overnight() {
        let w = BandwidthWindow {
            weekdays: [true; 7],
            start_minute: 22 * 60,
            end_minute: 6 * 60,
            limit_bps: 500_000,
        };
        assert!(w.is_active_at(0, 23 * 60));
        assert!(w.is_active_at(1, 3 * 60));
        assert!(!w.is_active_at(0, 20 * 60));
    }

    #[test]
    fn weekly_bandwidth_schedule_returns_lowest() {
        let schedule = WeeklyBandwidthSchedule {
            windows: vec![
                BandwidthWindow {
                    weekdays: [true; 7],
                    start_minute: 0,
                    end_minute: 24 * 60,
                    limit_bps: 2_000_000,
                },
                BandwidthWindow {
                    weekdays: [true; 7],
                    start_minute: 9 * 60,
                    end_minute: 17 * 60,
                    limit_bps: 500_000,
                },
            ],
        };
        assert_eq!(schedule.active_limit_bps(0, 12 * 60), Some(500_000));
        assert_eq!(schedule.active_limit_bps(0, 3 * 60), Some(2_000_000));
        assert_eq!(schedule.active_limit_bps(0, 18 * 60), Some(2_000_000));
    }

    #[test]
    fn zero_limit_bps_rejected_by_policy() {
        let mut policy = ResourcePolicy::default();
        policy.weekly_bandwidth_schedule.windows.push(BandwidthWindow {
            weekdays: [true; 7],
            start_minute: 0,
            end_minute: 60,
            limit_bps: 0,
        });
        assert_eq!(policy.validate(), Err(ResourcePolicyError::BandwidthLimit));
    }
}
