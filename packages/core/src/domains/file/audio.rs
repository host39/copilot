//! Audio files: the WAV container of spoken replies, and how long a WAV or MP3 file plays (for
//! the clip limit, `AUDIO_CLIP_MAX_SECS`).
//!
//! Durations follow what a decoder plays, never a field it ignores, and come without decoding:
//! - WAV: the `data` length over the sample rate times the frame size (channels times bytes per
//!   sample) of the one `fmt ` chunk, for PCM, IEEE float, A-law, and mu-law samples. A file
//!   whose byte rate or block alignment disagrees with that frame size, or with a second `fmt `
//!   chunk, has no duration.
//! - MP3: every Layer III frame from the first one counted, hopping by frame length and resyncing
//!   past anything that is not a frame, as decoders do; a Xing, Info, or VBRI frame count that
//!   says more wins.
//!
//! A file these do not describe has no duration.

use bytes::{BufMut, Bytes, BytesMut};

use crate::domains::file::media::MediaType;

/// Bytes after an ID3 tag searched for the first MP3 frame.
const MP3_SYNC_SEARCH: usize = 64 * 1024;
/// Length of the canonical WAV header [`wav`] writes.
const WAV_HEADER_BYTES: usize = 44;

/// A WAV file holding `pcm`: mono 16-bit little-endian samples at `sample_rate`. A trailing odd
/// byte (half a sample) is dropped.
pub fn wav(pcm: &[u8], sample_rate: u32) -> Bytes {
    let room = (u32::MAX as usize - WAV_HEADER_BYTES) & !1;
    let data = &pcm[..pcm.len().min(room) & !1];
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let mut file = BytesMut::with_capacity(WAV_HEADER_BYTES + data.len());
    file.put_slice(b"RIFF");
    file.put_u32_le(length + 36);
    file.put_slice(b"WAVE");
    file.put_slice(b"fmt ");
    file.put_u32_le(16);
    file.put_u16_le(1); // PCM
    file.put_u16_le(1); // mono
    file.put_u32_le(sample_rate);
    file.put_u32_le(sample_rate.saturating_mul(2));
    file.put_u16_le(2); // bytes per sample frame
    file.put_u16_le(16); // bits per sample
    file.put_slice(b"data");
    file.put_u32_le(length);
    file.put_slice(data);
    file.freeze()
}

/// How long an audio file plays, in milliseconds, when its container says.
pub fn duration_ms(media: MediaType, bytes: &[u8]) -> Option<u64> {
    match media {
        MediaType::Wav => wav_ms(bytes),
        MediaType::Mpeg => mp3_ms(bytes),
        _ => None,
    }
}

/// WAV format tags whose samples all take the same number of bytes.
const PCM: u16 = 0x0001;
const FLOAT: u16 = 0x0003;
const ALAW: u16 = 0x0006;
const MULAW: u16 = 0x0007;
/// `WAVE_FORMAT_EXTENSIBLE`: the format tag is the first two bytes of the sub-format GUID.
const EXTENSIBLE: u16 = 0xfffe;

/// The `data` chunk's length over the sample rate times the frame size of the file's one `fmt `
/// chunk. A `data` length of 0 or `0xFFFFFFFF` (a header written before the length was known)
/// or past the end of the file reads as the bytes that are there.
fn wav_ms(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut offset: usize = 12;
    let mut rate = None;
    while let Some(header) = offset.checked_add(8).and_then(|end| bytes.get(offset..end)) {
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let body = offset + 8;
        match &header[..4] {
            // Decoders read the first `fmt ` chunk and this used to read the last: a file with
            // two is not measured.
            b"fmt " if rate.is_some() => return None,
            b"fmt " => rate = Some(byte_rate(bytes.get(body..)?, size)?),
            b"data" => {
                let there = (bytes.len() - body) as u64;
                let length = match size {
                    0 | u32::MAX => there,
                    size => u64::from(size).min(there),
                };
                return Some(length * 1_000 / rate?);
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        offset = body
            .checked_add(size as usize)?
            .checked_add(size as usize & 1)?;
    }
    None
}

/// The bytes per second a `fmt ` chunk of `size` bytes plays: the sample rate times the frame
/// size, when every sample takes whole bytes and the stated byte rate and block alignment agree.
fn byte_rate(chunk: &[u8], size: u32) -> Option<u64> {
    let field = |at: usize| u16::from_le_bytes([chunk[at], chunk[at + 1]]);
    if size < 16 || chunk.len() < 16 {
        return None;
    }
    let mut tag = field(0);
    if tag == EXTENSIBLE {
        if size < 40 || chunk.len() < 26 {
            return None;
        }
        tag = field(24);
    }
    if !matches!(tag, PCM | FLOAT | ALAW | MULAW) {
        return None;
    }
    let channels = u64::from(field(2));
    let rate = u64::from(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
    let stated = u64::from(u32::from_le_bytes([
        chunk[8], chunk[9], chunk[10], chunk[11],
    ]));
    let align = u64::from(field(12));
    let bits = field(14);
    let frame = channels * u64::from(bits.div_ceil(8));
    let played = rate * frame;
    (bits > 0 && played > 0 && align == frame && stated == played).then_some(played)
}

/// An MPEG audio Layer III frame header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Frame {
    /// MPEG-1 (`true`) or MPEG-2 and 2.5.
    mpeg1: bool,
    mono: bool,
    padded: bool,
    kbps: u32,
    sample_rate: u32,
}

impl Frame {
    fn parse(header: &[u8]) -> Option<Self> {
        let &[0xff, second, third, fourth, ..] = header else {
            return None;
        };
        if second & 0xe0 != 0xe0 || (second >> 1) & 0x03 != 0x01 {
            return None;
        }
        let version = (second >> 3) & 0x03;
        let rates: [u32; 3] = match version {
            3 => [44_100, 48_000, 32_000],
            2 => [22_050, 24_000, 16_000],
            0 => [11_025, 12_000, 8_000],
            _ => return None,
        };
        let mpeg1 = version == 3;
        let kbps_table: [u32; 14] = if mpeg1 {
            [
                32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ]
        } else {
            [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160]
        };
        let kbps = *kbps_table.get(usize::from(third >> 4).checked_sub(1)?)?;
        let sample_rate = *rates.get(usize::from((third >> 2) & 0x03))?;
        Some(Self {
            mpeg1,
            mono: fourth >> 6 == 0x03,
            padded: (third >> 1) & 0x01 == 0x01,
            kbps,
            sample_rate,
        })
    }

    fn samples(self) -> u64 {
        if self.mpeg1 { 1_152 } else { 576 }
    }

    /// How long the frame plays, in microseconds.
    fn micros(self) -> u64 {
        self.samples() * 1_000_000 / u64::from(self.sample_rate)
    }

    /// The frame's length in bytes, header included (at least 24).
    fn length(self) -> usize {
        let per_kbps = if self.mpeg1 { 144_000 } else { 72_000 };
        (per_kbps * self.kbps / self.sample_rate + u32::from(self.padded)) as usize
    }

    /// The frame count of a Xing or Info header (after the side information) or a VBRI header
    /// (32 bytes after the frame header), when the frame holds one.
    fn frames(self, frame: &[u8]) -> Option<u64> {
        let side = match (self.mpeg1, self.mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        };
        let xing = frame.get(4 + side..)?;
        if xing.starts_with(b"Xing") || xing.starts_with(b"Info") {
            let flags = xing.get(4..8)?;
            if flags[3] & 0x01 == 0 {
                return None;
            }
            return xing.get(8..12).map(be);
        }
        let vbri = frame.get(36..)?;
        if vbri.starts_with(b"VBRI") {
            return vbri.get(14..18).map(be);
        }
        None
    }
}

fn be(bytes: &[u8]) -> u64 {
    u64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Skips an ID3v2 tag, finds the first frame, and counts the frames from there to the end (a
/// trailing ID3v1 tag does not count as audio). A Xing, Info, or VBRI frame count in the first
/// frame is taken when it says the file plays longer.
fn mp3_ms(bytes: &[u8]) -> Option<u64> {
    let mut start = 0;
    if let Some(tag) = bytes.get(..10).filter(|tag| tag.starts_with(b"ID3")) {
        let size = tag[6..10]
            .iter()
            .fold(0_usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
        let footer = if tag[5] & 0x10 == 0 { 0 } else { 10 };
        start = 10 + size + footer;
    }
    let search = bytes.get(start..)?;
    let (offset, first) = search
        .windows(4)
        .take(MP3_SYNC_SEARCH)
        .enumerate()
        .find_map(|(offset, header)| Frame::parse(header).map(|frame| (start + offset, frame)))?;
    let declared = first.frames(&bytes[offset..]).map_or(0, |frames| {
        frames * first.samples() * 1_000 / u64::from(first.sample_rate)
    });
    let mut end = bytes.len();
    if end >= offset + 128 && bytes[end - 128..].starts_with(b"TAG") {
        end -= 128;
    }
    Some(declared.max(counted(&bytes[offset..end]) / 1_000))
}

/// Microseconds of the Layer III frames in `bytes`: a frame's header leads to the next one, and
/// bytes that are not a frame header are skipped one at a time until one is, the way decoders
/// resync, so data placed between frames cannot hide the frames after it.
fn counted(bytes: &[u8]) -> u64 {
    let mut position = 0;
    let mut micros = 0;
    while let Some(header) = bytes.get(position..position + 4) {
        match Frame::parse(header) {
            Some(frame) => {
                micros += frame.micros();
                position += frame.length();
            }
            None => position += 1,
        }
    }
    micros
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An MPEG-1 Layer III, 128 kbps, 44.1 kHz, stereo frame header.
    const MPEG1_128K: [u8; 4] = [0xff, 0xfb, 0x90, 0x00];

    #[test]
    fn writes_a_mono_pcm16_wav_header() {
        let file = wav(&[1, 2, 3, 4, 5], 24_000);
        assert_eq!(file.len(), 44 + 4, "the odd byte is dropped");
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(file[4..8].try_into().unwrap()), 36 + 4);
        assert_eq!(&file[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([file[20], file[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([file[22], file[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 24_000);
        assert_eq!(u32::from_le_bytes(file[28..32].try_into().unwrap()), 48_000);
        assert_eq!(u16::from_le_bytes([file[34], file[35]]), 16);
        assert_eq!(&file[36..40], b"data");
        assert_eq!(u32::from_le_bytes(file[40..44].try_into().unwrap()), 4);
        assert_eq!(&file[44..], [1, 2, 3, 4]);
        assert_eq!(
            MediaType::detect("reply.wav", &file).ok(),
            Some(MediaType::Wav),
            "the upload check accepts it"
        );
    }

    #[test]
    fn measures_the_wav_files_it_writes() {
        let second = vec![0_u8; 48_000];
        assert_eq!(
            duration_ms(MediaType::Wav, &wav(&second, 24_000)),
            Some(1_000)
        );
        let clip = vec![0_u8; 16_000 * 2 * 3 / 2];
        assert_eq!(
            duration_ms(MediaType::Wav, &wav(&clip, 16_000)),
            Some(1_500)
        );
    }

    #[test]
    fn skips_other_chunks_and_trusts_the_bytes_over_the_header() {
        let mut file = b"RIFF\0\0\0\0WAVE".to_vec();
        file.extend_from_slice(b"LIST\x03\0\0\0abc\0"); // odd size, padded
        file.extend_from_slice(b"fmt \x10\0\0\0");
        file.extend_from_slice(&[1, 0, 1, 0]);
        file.extend_from_slice(&8_000_u32.to_le_bytes());
        file.extend_from_slice(&16_000_u32.to_le_bytes());
        file.extend_from_slice(&[2, 0, 16, 0]);
        let header = file.len();
        file.extend_from_slice(b"data");
        file.extend_from_slice(&u32::MAX.to_le_bytes());
        file.extend_from_slice(&[0; 8_000]);
        assert_eq!(
            duration_ms(MediaType::Wav, &file),
            Some(500),
            "streamed length"
        );

        file[header + 4..header + 8].copy_from_slice(&4_000_u32.to_le_bytes());
        assert_eq!(
            duration_ms(MediaType::Wav, &file),
            Some(250),
            "the stated length"
        );

        file[header + 4..header + 8].copy_from_slice(&1_000_000_u32.to_le_bytes());
        assert_eq!(duration_ms(MediaType::Wav, &file), Some(500), "cut off");
    }

    /// A WAV header whose `fmt ` chunk is `format`, then `data` of `length` bytes.
    fn wav_with(format: &[u8], length: usize) -> Vec<u8> {
        let mut file = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        file.extend_from_slice(&u32::try_from(format.len()).unwrap().to_le_bytes());
        file.extend_from_slice(format);
        file.extend_from_slice(b"data");
        file.extend_from_slice(&u32::try_from(length).unwrap().to_le_bytes());
        file.resize(file.len() + length, 0);
        file
    }

    /// A 16-byte `fmt ` body: tag, channels, rate, byte rate, block alignment, bits.
    fn format(
        tag: u16,
        channels: u16,
        rate: u32,
        byte_rate: u32,
        align: u16,
        bits: u16,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&tag.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&rate.to_le_bytes());
        body.extend_from_slice(&byte_rate.to_le_bytes());
        body.extend_from_slice(&align.to_le_bytes());
        body.extend_from_slice(&bits.to_le_bytes());
        body
    }

    #[test]
    fn measures_wav_by_the_frame_size_decoders_play() {
        // Stereo 32-bit float at 48 kHz: 384,000 bytes a second.
        let float = format(3, 2, 48_000, 384_000, 8, 32);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&float, 192_000)),
            Some(500)
        );
        // Mono mu-law at 8 kHz: one byte a sample.
        let mulaw = format(7, 1, 8_000, 8_000, 1, 8);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&mulaw, 16_000)),
            Some(2_000)
        );
        // Mono 24-bit PCM in an extensible header (sub-format PCM).
        let mut extensible = format(0xfffe, 1, 16_000, 48_000, 3, 24);
        extensible.extend_from_slice(&[22, 0, 24, 0, 4, 0, 0, 0]);
        extensible.extend_from_slice(&[
            1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
        ]);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&extensible, 96_000)),
            Some(2_000)
        );
    }

    #[test]
    fn a_forged_wav_header_has_no_duration() {
        // 20 MiB of 8 kHz mono PCM16 plays 1,310.72 seconds whatever the byte rate says.
        let length = 20 * 1_048_576;
        let honest = format(1, 1, 8_000, 16_000, 2, 16);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&honest, length)),
            Some(1_310_720)
        );
        let fast = format(1, 1, 8_000, 0x7fff_ffff, 2, 16);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&fast, length)),
            None,
            "byte rate"
        );
        let wide = format(1, 1, 8_000, 0x7fff_fffe, 0xffff, 16);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&wide, length)),
            None,
            "block alignment"
        );

        // A second `fmt ` chunk with that rate: decoders play the first one.
        let mut twice = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0".to_vec();
        twice.extend_from_slice(&honest);
        twice.extend_from_slice(b"fmt \x10\0\0\0");
        twice.extend_from_slice(&fast);
        twice.extend_from_slice(b"data");
        twice.extend_from_slice(&u32::try_from(length).unwrap().to_le_bytes());
        twice.resize(twice.len() + length, 0);
        assert_eq!(duration_ms(MediaType::Wav, &twice), None, "two fmt chunks");

        // Compressed samples (IMA ADPCM) do not take whole bytes each.
        let adpcm = format(0x11, 1, 8_000, 4_055, 256, 4);
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&adpcm, 4_096)),
            None,
            "ADPCM"
        );
        // A short `fmt ` chunk, and an extensible header without its sub-format.
        assert_eq!(
            duration_ms(MediaType::Wav, &wav_with(&honest[..14], 16)),
            None
        );
        let bare = format(0xfffe, 1, 8_000, 16_000, 2, 16);
        assert_eq!(duration_ms(MediaType::Wav, &wav_with(&bare, 16)), None);
    }

    #[test]
    fn a_wav_without_a_byte_rate_or_data_has_no_duration() {
        assert_eq!(duration_ms(MediaType::Wav, b"RIFF\x10\0\0\0WAVEfmt "), None);
        let mut file = wav(&[0; 4], 0).to_vec();
        assert_eq!(duration_ms(MediaType::Wav, &file), None, "zero byte rate");
        file.truncate(36);
        assert_eq!(duration_ms(MediaType::Wav, &file), None, "no data chunk");
        assert_eq!(duration_ms(MediaType::Wav, b"RIFF"), None);
        assert_eq!(duration_ms(MediaType::Png, &wav(&[0; 4], 8_000)), None);
    }

    /// `count` frames of `header`, each the length the header states.
    fn frames(header: [u8; 4], count: usize) -> Vec<u8> {
        let length = Frame::parse(&header).expect("a frame header").length();
        let mut frame = header.to_vec();
        frame.resize(length, 0);
        frame.repeat(count)
    }

    /// Milliseconds of `count` MPEG-1, 44.1 kHz frames, as the counter adds them.
    fn mpeg1_ms(count: u64) -> u64 {
        count * (1_152 * 1_000_000 / 44_100) / 1_000
    }

    #[test]
    fn counts_every_mp3_frame() {
        assert_eq!(Frame::parse(&MPEG1_128K).map(Frame::length), Some(417));
        let file = frames(MPEG1_128K, 383);
        assert_eq!(duration_ms(MediaType::Mpeg, &file), Some(mpeg1_ms(383)));
        assert_eq!(mpeg1_ms(383), 10_004);

        // An ID3v2 tag (syncsafe size 0x0101 = 129 bytes), and an ID3v1 tag at the end.
        let mut tagged = b"ID3\x04\0\0\0\0\x01\x01".to_vec();
        tagged.resize(10 + 129, 0);
        tagged.extend_from_slice(&file);
        tagged.extend_from_slice(b"TAG");
        tagged.resize(tagged.len() + 125, 0);
        assert_eq!(duration_ms(MediaType::Mpeg, &tagged), Some(10_004));

        // A padded frame is one byte longer.
        let padded = frames([0xff, 0xfb, 0x92, 0x00], 2);
        assert_eq!(padded.len(), 2 * 418);
        assert_eq!(duration_ms(MediaType::Mpeg, &padded), Some(mpeg1_ms(2)));
    }

    #[test]
    fn data_between_mp3_frames_hides_none_of_them() {
        let mut file = frames(MPEG1_128K, 100);
        file.extend_from_slice(&[0x55; 1_001]);
        file.extend_from_slice(&frames(MPEG1_128K, 100));
        file.extend_from_slice(&[0xff; 3]);
        file.extend_from_slice(&frames(MPEG1_128K, 100));
        assert_eq!(duration_ms(MediaType::Mpeg, &file), Some(mpeg1_ms(300)));
    }

    #[test]
    fn a_forged_frame_count_never_shortens_an_mp3() {
        // A Xing frame that claims 10 frames, then 383 more: every frame plays.
        let mut file = MPEG1_128K.to_vec();
        file.resize(4 + 32, 0);
        file.extend_from_slice(b"Xing\0\0\0\x01");
        file.extend_from_slice(&10_u32.to_be_bytes());
        file.resize(417, 0);
        file.extend_from_slice(&frames(MPEG1_128K, 383));
        assert_eq!(duration_ms(MediaType::Mpeg, &file), Some(mpeg1_ms(384)));
    }

    #[test]
    fn reads_the_frame_count_of_xing_and_vbri_headers() {
        // 100 MPEG-1 frames of 1,152 samples at 44.1 kHz.
        let mut xing = MPEG1_128K.to_vec();
        xing.resize(4 + 32, 0);
        xing.extend_from_slice(b"Xing\0\0\0\x01");
        xing.extend_from_slice(&100_u32.to_be_bytes());
        xing.resize(4_000, 0);
        assert_eq!(duration_ms(MediaType::Mpeg, &xing), Some(2_612));

        // MPEG-2, 24 kHz, mono: 576 samples per frame, side information of 9 bytes.
        let mut info = vec![0xff, 0xf3, 0x14, 0xc0];
        info.resize(4 + 9, 0);
        info.extend_from_slice(b"Info\0\0\0\x01");
        info.extend_from_slice(&250_u32.to_be_bytes());
        info.resize(2_000, 0);
        assert_eq!(duration_ms(MediaType::Mpeg, &info), Some(6_000));

        let mut vbri = MPEG1_128K.to_vec();
        vbri.resize(36, 0);
        vbri.extend_from_slice(b"VBRI\0\x01\0\0\0\0\0\0\0\0");
        vbri.extend_from_slice(&441_u32.to_be_bytes());
        vbri.resize(4_000, 0);
        assert_eq!(duration_ms(MediaType::Mpeg, &vbri), Some(11_520));
    }

    #[test]
    fn mp3_without_a_frame_has_no_duration() {
        assert_eq!(duration_ms(MediaType::Mpeg, b"ID3\x04\0\0\0\0\0\0"), None);
        assert_eq!(
            duration_ms(MediaType::Mpeg, &[0xff, 0xfb, 0xf0, 0]),
            None,
            "bad bitrate"
        );
        assert_eq!(
            duration_ms(MediaType::Mpeg, &[0xff, 0xfb, 0x0c, 0]),
            None,
            "free format"
        );
        assert_eq!(
            duration_ms(MediaType::Mpeg, &[0xff, 0xfb, 0x9c, 0]),
            None,
            "bad rate"
        );
        assert_eq!(
            duration_ms(MediaType::Mpeg, &[0xff, 0xe9, 0x90, 0]),
            None,
            "reserved"
        );
    }
}
