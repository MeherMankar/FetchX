// Adapted from ffmpreg (Apache-2.0)
use crate::{
    Packet, Parameters, Result,
    frame::{Channels, SampleFormat, SampleLayout},
    state::{AudioState, CodecState, State},
    traits::{DecodeOut, Decoder},
};

pub struct AdpcmDecoder;

impl Decoder for AdpcmDecoder {
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
        todo!()
    }

    fn flush(&self, _state: &mut State) -> Result<DecodeOut<State>> {
        todo!()
    }
}
