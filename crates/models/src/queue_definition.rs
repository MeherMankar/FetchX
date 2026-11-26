use serde::{Deserialize, Serialize};

use crate::ResourcePolicy;

/// A named download queue with optional resource constraints.
///
/// `max_active` and `bandwidth_limit_bps` override the global resource policy
/// for downloads belonging to this queue. `None` means inherit global settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueueDefinition {
    /// UUID string identifying this queue.
    pub id: String,
    /// Human-readable name, e.g. "Night", "Video", "Work".
    pub name: String,
    /// Maximum concurrent active downloads in this queue. `None` = inherit global.
    pub max_active: Option<u32>,
    /// Bandwidth cap for all downloads in this queue (bytes/s). `None` = unlimited.
    pub bandwidth_limit_bps: Option<u64>,
    /// Whether this queue dispatches downloads. Disabled queues hold all jobs queued.
    pub enabled: bool,
}

/// A named snapshot of a [`ResourcePolicy`] that can be applied to a queue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueueProfile {
    /// UUID string identifying this profile.
    pub id: String,
    /// Human-readable name, e.g. "Night", "Day", "Unlimited".
    pub name: String,
    /// The resource policy captured by this profile.
    pub resource_policy: ResourcePolicy,
}
