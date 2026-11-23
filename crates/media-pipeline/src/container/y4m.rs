// Adapted from ffmpreg (Apache-2.0)
use crate::{
    Io, Result,
    stream::Hint,
    traits::{Demuxer, Muxer, Probe},
};

const Y4M_MAGIC: &[u8] = b"YUV4MPEG2";

pub struct Y4mProbe;

impl Probe for Y4mProbe {
    fn demuxer_matches(data: &[u8]) -> bool {
        if data.len() < Y4M_MAGIC.len() {
            return false;
        }
        &data[0..Y4M_MAGIC.len()] == Y4M_MAGIC
    }

    fn muxer_matches(hint: &Hint) -> bool {
        if let Some(mime) = hint.mime_type.as_ref().map(|x| x.trim()) {
            if mime == "video/x-yuv4mpegpipe" {
                return true;
            }
        }
        hint.extension.as_ref().is_some_and(|e| e == "y4m")
    }

    fn create_demuxer(_io: Box<dyn Io>) -> Result<Box<dyn Demuxer>> {
        Err(crate::Message::Container("Y4M demuxing is not implemented"))
    }

    fn create_muxer(_io: Box<dyn Io>) -> Result<Box<dyn Muxer>> {
        Err(crate::Message::Container("Y4M muxing is not implemented"))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hint, Io, Message, traits::Probe};

    use super::Y4mProbe;

    struct EmptyIo;

    impl Io for EmptyIo {
        fn read(&mut self, _buffer: &mut [u8]) -> crate::Result<usize> {
            Ok(0)
        }

        fn write(&mut self, buffer: &[u8]) -> crate::Result<usize> {
            Ok(buffer.len())
        }
    }

    #[test]
    fn unsupported_factories_return_errors_instead_of_panicking() {
        assert!(matches!(
            Y4mProbe::create_demuxer(Box::new(EmptyIo)),
            Err(Message::Container("Y4M demuxing is not implemented"))
        ));
        assert!(matches!(
            Y4mProbe::create_muxer(Box::new(EmptyIo)),
            Err(Message::Container("Y4M muxing is not implemented"))
        ));
        assert!(Y4mProbe::muxer_matches(&Hint {
            extension: Some("y4m".to_owned()),
            ..Hint::default()
        }));
    }
}
