// Adapted from ffmpreg (Apache-2.0)
use crate::{
    Packet, Parameters, Result,
    frame::{Channels, SampleFormat, SampleLayout},
    state::{AudioState, CodecState, State},
    traits::{DecodeOut, Decoder},
};

pub struct PcmDecoder;

impl Decoder for PcmDecoder {
    fn create_state(&self, p: &Parameters) -> State {
        let codec_state = CodecState {
            extradata: Vec::new(),
            pts: None,
            dts: None,
            last_pts: None,
        };
        let audio_state = AudioState {
            sample_rate: p.sample_rate.unwrap_or(44100),
            channels: Channels::from_value(p.channels.unwrap_or(2)),
            format: SampleFormat::S16,
            layout: SampleLayout::Interleaved,
            frame_size: 0,
            codec_state,
            samples_consumed: 0,
            samples_produced: 0,
            buffer: Vec::new(),
        };
        State::Audio(audio_state)
    }

    fn decode(&self, _state: &mut State, _packet: Packet) -> Result<DecodeOut<State>> {
        Err(crate::Message::Codec("PCM decoding is not implemented"))
    }

    fn flush(&self, _state: &mut State) -> Result<DecodeOut<State>> {
        Err(crate::Message::Codec("PCM decoding is not implemented"))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Message, Packet, State, StreamId, traits::Decoder};

    use super::PcmDecoder;

    #[test]
    fn unsupported_decode_and_flush_return_errors_instead_of_panicking() {
        let decoder = PcmDecoder;
        let mut state = decoder.create_state(&crate::stream::Parameters {
            codec: crate::CodecId::new("pcm_s16le"),
            profile: None,
            bitrate: None,
            sample_rate: Some(44_100),
            channels: Some(2),
            width: None,
            height: None,
            pixel_format: None,
        });

        assert!(matches!(
            decoder.decode(&mut state, Packet::new(StreamId(0), vec![0, 0])),
            Err(Message::Codec("PCM decoding is not implemented"))
        ));
        assert!(matches!(
            decoder.flush(&mut state),
            Err(Message::Codec("PCM decoding is not implemented"))
        ));
        assert!(matches!(state, State::Audio(_)));
    }
}
