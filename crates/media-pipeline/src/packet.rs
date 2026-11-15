// Adapted from ffmpreg (Apache-2.0)
use crate::stream::StreamId;
use crate::time::Time;

#[derive(Debug, Clone)]
pub struct Packet {
    pub stream_id: StreamId,
    pub data: Vec<u8>,
    pub pts: Option<Time>,
    pub dts: Option<Time>,
    pub duration: Option<Time>,
}

impl Packet {
    pub fn new(stream_id: StreamId, data: Vec<u8>) -> Self {
        Self { stream_id, data, pts: None, dts: None, duration: None }
    }
}
