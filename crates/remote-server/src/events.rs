use fetchx_models::DownloadStatus;
use fetchx_queue::DownloadEvent;
use fetchx_remote_protocol::{EventName, EventNotification};
use serde_json::{Value, json};

use crate::dispatch::safe_download_event;

pub(crate) fn notifications(event: DownloadEvent) -> Vec<EventNotification> {
    match event {
        DownloadEvent::Created { revision, item } => {
            let data = safe_download_event(&item, revision);
            vec![
                EventNotification::new(EventName::DownloadCreated, data.clone()),
                EventNotification::new(EventName::QueueUpdated, data),
            ]
        }
        DownloadEvent::Updated { revision, item } => {
            let data = safe_download_event(&item, revision);
            let mut events = vec![
                EventNotification::new(EventName::DownloadUpdated, data.clone()),
                EventNotification::new(EventName::DownloadProgress, data.clone()),
            ];
            let lifecycle_event = match item.status() {
                DownloadStatus::Paused => Some(EventName::DownloadPaused),
                DownloadStatus::Completed => Some(EventName::DownloadCompleted),
                DownloadStatus::Failed => Some(EventName::DownloadFailed),
                _ => None,
            };
            if let Some(event) = lifecycle_event {
                events.push(EventNotification::new(event, data.clone()));
            }
            events.push(EventNotification::new(EventName::QueueUpdated, data));
            events
        }
        DownloadEvent::Removed { revision, id } => vec![EventNotification::new(
            EventName::QueueUpdated,
            json!({ "id": id, "revision": revision, "removed": true }),
        )],
    }
}

pub(crate) fn serialized(notification: &EventNotification) -> Result<Value, serde_json::Error> {
    serde_json::to_value(notification)
}
