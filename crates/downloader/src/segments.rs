use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use reqwest::{
    Client, StatusCode, Url,
    header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE, RETRY_AFTER},
};
use thiserror::Error;
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt, SeekFrom},
    sync::watch,
};

use crate::DownloadEngineError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SegmentState {
    Pending,
    Downloading,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Segment {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub current: u64,
    pub expected_length: u64,
    pub state: SegmentState,
    pub attempts: u8,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SegmentResult {
    Complete,
    Fallback,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum SegmentPlanError {
    #[error("segment byte range overflowed")]
    RangeOverflow,
}

pub(super) fn plan_segments(
    total_size: u64,
    count: usize,
    path_for_index: impl Fn(usize) -> PathBuf,
) -> Result<Option<Vec<Segment>>, SegmentPlanError> {
    if total_size == 0 || !(2..=16).contains(&count) {
        return Ok(None);
    }

    let count_u64 = u64::try_from(count).map_err(|_| SegmentPlanError::RangeOverflow)?;
    let count_u64 = count_u64.min(total_size);
    if count_u64 < 2 {
        return Ok(None);
    }
    let count = usize::try_from(count_u64).map_err(|_| SegmentPlanError::RangeOverflow)?;
    let base_length = total_size / count_u64;
    let remainder = total_size % count_u64;
    let mut segments = Vec::with_capacity(count);
    let mut start = 0_u64;

    for index in 0..count {
        let extra = u64::from(
            u64::try_from(index).map_err(|_| SegmentPlanError::RangeOverflow)? < remainder,
        );
        let expected_length = base_length + extra;
        let end = start
            .checked_add(expected_length)
            .and_then(|value| value.checked_sub(1))
            .ok_or(SegmentPlanError::RangeOverflow)?;
        segments.push(Segment {
            index,
            start,
            end,
            current: 0,
            expected_length,
            state: SegmentState::Pending,
            attempts: 0,
            path: path_for_index(index),
        });
        start = end.checked_add(1).ok_or(SegmentPlanError::RangeOverflow)?;
    }

    if start != total_size {
        return Err(SegmentPlanError::RangeOverflow);
    }
    Ok(Some(segments))
}

pub(super) async fn prepare_segment(segment: &mut Segment) -> Result<bool, DownloadEngineError> {
    match fs::symlink_metadata(&segment.path).await {
        Ok(metadata) if metadata.file_type().is_file() => {
            segment.current = metadata.len();
            if segment.current > segment.expected_length {
                return Ok(false);
            }
        }
        Ok(_) => return Err(segment_error(segment)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            segment.current = 0;
        }
        Err(error) => return Err(DownloadEngineError::Io(error)),
    }

    segment.state = if segment.current == segment.expected_length {
        SegmentState::Complete
    } else {
        SegmentState::Pending
    };
    Ok(true)
}

pub(super) async fn download_segment(
    segment: &mut Segment,
    total_size: u64,
    requested_url: &Url,
    client: &Client,
    cancellation: &mut watch::Receiver<bool>,
    stop: &mut watch::Receiver<bool>,
    global_downloaded: &AtomicU64,
) -> Result<SegmentResult, DownloadEngineError> {
    if segment.state == SegmentState::Complete {
        return Ok(SegmentResult::Complete);
    }

    segment.state = SegmentState::Downloading;
    segment.attempts = segment.attempts.saturating_add(1);
    let mut file = if segment.current == 0 {
        match fs::symlink_metadata(&segment.path).await {
            Ok(metadata) if metadata.file_type().is_file() && metadata.len() == 0 => {
                OpenOptions::new()
                    .write(true)
                    .open(&segment.path)
                    .await
                    .map_err(|_| segment_error(segment))?
            }
            Ok(_) => return Err(segment_error(segment)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&segment.path)
                .await
                .map_err(|_| segment_error(segment))?,
            Err(error) => return Err(DownloadEngineError::Io(error)),
        }
    } else {
        let path_metadata = fs::symlink_metadata(&segment.path)
            .await
            .map_err(DownloadEngineError::Io)?;
        if !path_metadata.file_type().is_file() || path_metadata.len() != segment.current {
            return Err(segment_error(segment));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&segment.path)
            .await
            .map_err(DownloadEngineError::Io)?;
        if file
            .metadata()
            .await
            .map_err(DownloadEngineError::Io)?
            .len()
            != segment.current
        {
            return Err(segment_error(segment));
        }
        file.seek(SeekFrom::End(0))
            .await
            .map_err(DownloadEngineError::Io)?;
        file
    };

    while segment.current < segment.expected_length {
        if is_cancelled(cancellation) || is_cancelled(stop) {
            file.sync_all().await.map_err(DownloadEngineError::Io)?;
            return Ok(SegmentResult::Cancelled);
        }

        let range_start = segment
            .start
            .checked_add(segment.current)
            .ok_or_else(|| segment_error(segment))?;
        let range_value = format!("bytes={range_start}-{}", segment.end);
        let request = client
            .get(requested_url.clone())
            .header(RANGE, range_value)
            .send();
        let response = tokio::select! {
            changed = cancellation.changed() => {
                if changed.is_err() || is_cancelled(cancellation) {
                    file.sync_all().await.map_err(DownloadEngineError::Io)?;
                    return Ok(SegmentResult::Cancelled);
                }
                continue;
            }
            changed = stop.changed() => {
                if changed.is_err() || is_cancelled(stop) {
                    file.sync_all().await.map_err(DownloadEngineError::Io)?;
                    return Ok(SegmentResult::Cancelled);
                }
                continue;
            }
            response = request => response.map_err(|_| segment_error(segment))?,
        };

        if response.status() == StatusCode::OK {
            return Ok(SegmentResult::Fallback);
        }
        if response.status() == StatusCode::TOO_MANY_REQUESTS
            || (response.status() == StatusCode::SERVICE_UNAVAILABLE
                && response.headers().contains_key(RETRY_AFTER))
        {
            return Err(DownloadEngineError::HttpStatus(response.status().as_u16()));
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            if matches!(
                response.status(),
                StatusCode::RANGE_NOT_SATISFIABLE | StatusCode::NOT_IMPLEMENTED
            ) {
                return Ok(SegmentResult::Fallback);
            }
            return Err(segment_error(segment));
        }
        if !same_http_target(response.url(), requested_url) {
            return Ok(SegmentResult::Fallback);
        }
        let Some((response_start, response_end, response_total)) = parse_content_range(&response)
        else {
            return Ok(SegmentResult::Fallback);
        };
        if response_start != range_start
            || response_end < response_start
            || response_end > segment.end
            || response_total != Some(total_size)
        {
            return Ok(SegmentResult::Fallback);
        }
        let expected_response_bytes = response_end
            .checked_sub(response_start)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| segment_error(segment))?;
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .is_some_and(|length| {
                length
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_none_or(|length| length != expected_response_bytes)
            })
        {
            return Ok(SegmentResult::Fallback);
        }

        let mut response_bytes = 0_u64;
        let mut response = response;
        loop {
            let next_chunk = tokio::select! {
                changed = cancellation.changed() => {
                    if changed.is_err() || is_cancelled(cancellation) {
                        file.sync_all().await.map_err(DownloadEngineError::Io)?;
                        return Ok(SegmentResult::Cancelled);
                    }
                    continue;
                }
                changed = stop.changed() => {
                    if changed.is_err() || is_cancelled(stop) {
                        file.sync_all().await.map_err(DownloadEngineError::Io)?;
                        return Ok(SegmentResult::Cancelled);
                    }
                    continue;
                }
                chunk = response.chunk() => chunk.map_err(|_| segment_error(segment))?,
            };
            let Some(chunk) = next_chunk else {
                break;
            };
            let chunk_size = u64::try_from(chunk.len()).map_err(|_| segment_error(segment))?;
            let next_response_bytes = response_bytes
                .checked_add(chunk_size)
                .ok_or_else(|| segment_error(segment))?;
            let next_segment_bytes = segment
                .current
                .checked_add(chunk_size)
                .ok_or_else(|| segment_error(segment))?;
            if next_response_bytes > expected_response_bytes
                || next_segment_bytes > segment.expected_length
            {
                return Err(segment_error(segment));
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| segment_error(segment))?;
            segment.current = next_segment_bytes;
            response_bytes = next_response_bytes;
            global_downloaded.fetch_add(chunk_size, Ordering::AcqRel);
        }
        if response_bytes != expected_response_bytes {
            return Err(segment_error(segment));
        }
    }

    file.flush().await.map_err(DownloadEngineError::Io)?;
    file.sync_all().await.map_err(DownloadEngineError::Io)?;
    segment.state = SegmentState::Complete;
    Ok(SegmentResult::Complete)
}

fn is_cancelled(receiver: &watch::Receiver<bool>) -> bool {
    *receiver.borrow()
}

fn parse_content_range(response: &reqwest::Response) -> Option<(u64, u64, Option<u64>)> {
    let value = response
        .headers()
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((
        start.parse().ok()?,
        end.parse().ok()?,
        (total != "*").then(|| total.parse().ok()).flatten(),
    ))
}

fn same_http_target(response: &Url, requested: &Url) -> bool {
    let mut response = response.clone();
    let mut requested = requested.clone();
    response.set_fragment(None);
    requested.set_fragment(None);
    response == requested
}

fn segment_error(segment: &Segment) -> DownloadEngineError {
    DownloadEngineError::SegmentFailed {
        index: segment.index,
        start: segment.start,
        end: segment.end,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{SegmentState, plan_segments};

    #[test]
    fn four_segments_cover_the_entire_file_without_gaps_or_overlaps() {
        let segments = plan_segments(1000, 4, |index| {
            PathBuf::from(format!("segment-{index}.part"))
        })
        .expect("valid range arithmetic")
        .expect("a nonempty file is segmentable");

        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.index, segment.start, segment.end))
                .collect::<Vec<_>>(),
            [(0, 0, 249), (1, 250, 499), (2, 500, 749), (3, 750, 999)]
        );
        assert_eq!(segments[0].state, SegmentState::Pending);
        assert_eq!(segments[0].expected_length, 250);
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.expected_length)
                .sum::<u64>(),
            1000
        );
    }

    #[test]
    fn uneven_files_distribute_the_remainder_to_early_segments() {
        let segments = plan_segments(11, 4, |_| PathBuf::new())
            .expect("valid range arithmetic")
            .expect("a nonempty file is segmentable");

        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.start, segment.end))
                .collect::<Vec<_>>(),
            [(0, 2), (3, 5), (6, 8), (9, 10)]
        );
    }

    #[test]
    fn small_or_invalid_plans_do_not_segment() {
        assert!(
            plan_segments(0, 4, |_| PathBuf::new())
                .expect("valid plan")
                .is_none()
        );
        assert!(
            plan_segments(100, 1, |_| PathBuf::new())
                .expect("invalid count selects fallback")
                .is_none()
        );
        assert!(
            plan_segments(100, 17, |_| PathBuf::new())
                .expect("unbounded count selects fallback")
                .is_none()
        );
    }
}
