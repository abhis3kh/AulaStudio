//! Windows playback loopback. Audio stays in memory on the owning worker thread.

use std::{marker::PhantomData, rc::Rc};
use windows::Win32::{
    Media::{
        Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole,
            eRender,
        },
        KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
        Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
    },
    System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Encoding {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

#[derive(Clone, Copy, Debug)]
struct MixFormat {
    sample_rate: u32,
    channels: u16,
    block_align: usize,
    encoding: Encoding,
}

impl MixFormat {
    fn parse(base: WAVEFORMATEX, extension: Option<WAVEFORMATEXTENSIBLE>) -> Result<Self, String> {
        let channels = base.nChannels;
        let sample_rate = base.nSamplesPerSec;
        let bits = base.wBitsPerSample;
        if channels == 0 || channels > 32 || !(8_000..=384_000).contains(&sample_rate) {
            return Err(
                "The playback device uses an unsupported channel count or sample rate.".into(),
            );
        }
        let tag = u32::from(base.wFormatTag);
        let (is_float, valid_bits) = if tag == WAVE_FORMAT_EXTENSIBLE {
            let extension =
                extension.ok_or("The playback device returned an incomplete audio format.")?;
            let subtype = extension.SubFormat;
            // WAVEFORMATEXTENSIBLE specifies which member of this union is meaningful.
            let valid_bits = unsafe { extension.Samples.wValidBitsPerSample };
            if subtype == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                (true, valid_bits)
            } else if subtype == KSDATAFORMAT_SUBTYPE_PCM {
                (false, valid_bits)
            } else {
                return Err("The playback device uses an unsupported audio encoding.".into());
            }
        } else if tag == WAVE_FORMAT_IEEE_FLOAT {
            (true, bits)
        } else if tag == WAVE_FORMAT_PCM {
            (false, bits)
        } else {
            return Err("The playback device uses an unsupported audio encoding.".into());
        };
        if valid_bits == 0 || valid_bits > bits || (is_float && valid_bits != 32) {
            return Err("The playback device returned an invalid sample precision.".into());
        }
        let encoding = match (is_float, bits) {
            (true, 32) => Encoding::Float32,
            (false, 16) => Encoding::Pcm16,
            (false, 24) => Encoding::Pcm24,
            (false, 32) => Encoding::Pcm32,
            _ => return Err("The playback device needs float32 or PCM16/24/32 audio.".into()),
        };
        let block_align = usize::from(channels) * usize::from(bits / 8);
        if usize::from(base.nBlockAlign) != block_align {
            return Err("The playback device returned an invalid audio frame size.".into());
        }
        Ok(Self {
            sample_rate,
            channels,
            block_align,
            encoding,
        })
    }

    fn decode(self, bytes: &[u8], output: &mut Vec<f32>) -> Result<(), String> {
        if !bytes.len().is_multiple_of(self.block_align) {
            return Err("The playback device returned a truncated audio frame.".into());
        }
        let sample_bytes = self.block_align / usize::from(self.channels);
        output.reserve(bytes.len() / self.block_align);
        for frame in bytes.chunks_exact(self.block_align) {
            let sum: f32 = frame
                .chunks_exact(sample_bytes)
                .map(|sample| {
                    let value = match self.encoding {
                        Encoding::Float32 => {
                            f32::from_le_bytes(sample.try_into().expect("four-byte sample"))
                        }
                        Encoding::Pcm16 => {
                            f32::from(i16::from_le_bytes(
                                sample.try_into().expect("two-byte sample"),
                            )) / 32768.0
                        }
                        Encoding::Pcm24 => {
                            let value = i32::from_le_bytes([
                                sample[0],
                                sample[1],
                                sample[2],
                                if sample[2] & 0x80 == 0 { 0 } else { 0xff },
                            ]);
                            value as f32 / 8_388_608.0
                        }
                        Encoding::Pcm32 => {
                            i32::from_le_bytes(sample.try_into().expect("four-byte sample")) as f32
                                / 2_147_483_648.0
                        }
                    };
                    if value.is_finite() {
                        value.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    }
                })
                .sum();
            output.push(sum / f32::from(self.channels));
        }
        Ok(())
    }
}

struct ComApartment(PhantomData<Rc<()>>);

impl ComApartment {
    fn open() -> Result<Self, String> {
        // This object is !Send and !Sync, so COM initialization and cleanup stay on one thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| audio_error("Initialize Windows audio", error))?;
        Ok(Self(PhantomData))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // Every successful CoInitializeEx, including S_FALSE, requires this matching call.
        unsafe { CoUninitialize() };
    }
}

struct AllocatedMixFormat(*mut WAVEFORMATEX);

impl Drop for AllocatedMixFormat {
    fn drop(&mut self) {
        // GetMixFormat returns a COM task-allocator allocation, independent of the client lifetime.
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

/// Open, poll, and drop this on the same background thread. Never captures a microphone.
pub struct LoopbackCapture {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    format: MixFormat,
    buffer_frames: u32,
    // Fields drop in declaration order. COM must outlive all client interfaces above.
    _apartment: ComApartment,
}

impl LoopbackCapture {
    pub fn open() -> Result<Self, String> {
        let apartment = ComApartment::open()?;
        // All COM calls use interfaces created in this apartment. The eRender endpoint and
        // LOOPBACK flag select system playback, never the default capture endpoint.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|error| audio_error("Find Windows playback devices", error))?;
        let endpoint = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(|error| audio_error("Open the default playback device", error))?;
        let client: IAudioClient = unsafe { endpoint.Activate(CLSCTX_ALL, None) }
            .map_err(|error| audio_error("Activate playback capture", error))?;
        let allocated = AllocatedMixFormat(
            unsafe { client.GetMixFormat() }
                .map_err(|error| audio_error("Read the playback format", error))?,
        );
        if allocated.0.is_null() {
            return Err("The playback device returned an empty audio format.".into());
        }
        // WASAPI owns the allocation and guarantees WAVEFORMATEX plus cbSize bytes. Packed
        // structs are copied with read_unaligned before reading fields or the extension.
        let base = unsafe { allocated.0.read_unaligned() };
        let extension = if u32::from(base.wFormatTag) == WAVE_FORMAT_EXTENSIBLE {
            if usize::from(base.cbSize)
                < std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>()
            {
                return Err("The playback device returned an incomplete extensible format.".into());
            }
            Some(unsafe { allocated.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() })
        } else {
            None
        };
        let format = MixFormat::parse(base, extension)?;
        // Shared-mode loopback uses the endpoint's unmodified native mix format. Polling
        // avoids an event handle and returns immediately when no packets are available.
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                1_000_000,
                0,
                allocated.0,
                None,
            )
        }
        .map_err(|error| audio_error("Initialize playback capture", error))?;
        let buffer_frames = unsafe { client.GetBufferSize() }
            .map_err(|error| audio_error("Read the playback buffer size", error))?;
        if buffer_frames == 0 || buffer_frames > format.sample_rate * 2 {
            return Err("The playback device returned an invalid buffer size.".into());
        }
        let capture = unsafe { client.GetService::<IAudioCaptureClient>() }
            .map_err(|error| audio_error("Open playback audio packets", error))?;
        unsafe { client.Start() }.map_err(|error| audio_error("Start playback capture", error))?;
        Ok(Self {
            capture,
            client,
            format,
            buffer_frames,
            _apartment: apartment,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.format.sample_rate
    }

    /// Appends currently available mono samples. Bounded to 32 packets or a quarter second
    /// of new samples per poll, so a busy endpoint cannot monopolize the worker.
    pub fn read_mono(&mut self, samples: &mut Vec<f32>) -> Result<(), String> {
        let initial_len = samples.len();
        let max_frames = self.format.sample_rate as usize / 4;
        for _ in 0..32 {
            let added = samples.len() - initial_len;
            if added >= max_frames {
                break;
            }
            let next = unsafe { self.capture.GetNextPacketSize() }
                .map_err(|error| audio_error("Poll playback audio", error))?;
            if next == 0 {
                break;
            }
            if next as usize > max_frames {
                return Err("The playback device returned an oversized audio packet.".into());
            }
            if added + next as usize > max_frames {
                break;
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0;
            let mut flags = 0;
            unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            }
            .map_err(|error| audio_error("Read playback audio", error))?;
            let packet = Packet {
                capture: &self.capture,
                frames,
                released: false,
            };
            if frames > self.buffer_frames || frames as usize > max_frames - added {
                return Err("The playback device returned an oversized audio packet.".into());
            }
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                samples.resize(samples.len() + frames as usize, 0.0);
            } else if frames != 0 {
                if data.is_null() {
                    return Err("The playback device returned an empty audio packet.".into());
                }
                let byte_count = frames as usize * self.format.block_align;
                // GetBuffer guarantees frames * nBlockAlign readable bytes until ReleaseBuffer.
                // buffer_frames and format validation above bound this slice and its decoding.
                let bytes = unsafe { std::slice::from_raw_parts(data.cast_const(), byte_count) };
                self.format.decode(bytes, samples)?;
            }
            packet.release()?;
            if frames == 0 {
                break;
            }
        }
        Ok(())
    }
}

impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        // Release resources even if the endpoint was unplugged. Stop can then report invalidation;
        // there is no caller to return it to during Drop, and interfaces still release normally.
        let _ = unsafe { self.client.Stop() };
    }
}

struct Packet<'a> {
    capture: &'a IAudioCaptureClient,
    frames: u32,
    released: bool,
}

impl Packet<'_> {
    fn release(mut self) -> Result<(), String> {
        self.released = true;
        unsafe { self.capture.ReleaseBuffer(self.frames) }
            .map_err(|error| audio_error("Release playback audio", error))
    }
}

impl Drop for Packet<'_> {
    fn drop(&mut self) {
        if !self.released {
            // Error paths still return the borrowed packet before destroying its client.
            let _ = unsafe { self.capture.ReleaseBuffer(self.frames) };
        }
    }
}

fn audio_error(operation: &str, error: windows::core::Error) -> String {
    if error.code() == AUDCLNT_E_DEVICE_INVALIDATED {
        "The playback device changed or disconnected. Stop and start music sync again.".into()
    } else {
        format!("{operation}: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(bits: u16, channels: u16, float: bool) -> MixFormat {
        MixFormat::parse(
            WAVEFORMATEX {
                wFormatTag: if float {
                    WAVE_FORMAT_IEEE_FLOAT as u16
                } else {
                    WAVE_FORMAT_PCM as u16
                },
                nChannels: channels,
                nSamplesPerSec: 48_000,
                nBlockAlign: channels * (bits / 8),
                wBitsPerSample: bits,
                ..Default::default()
            },
            None,
        )
        .unwrap()
    }

    #[test]
    fn pcm16_stereo_is_averaged_and_signed() {
        let mut out = Vec::new();
        format(16, 2, false)
            .decode(&[0x00, 0x40, 0x00, 0xc0, 0x00, 0x80, 0x00, 0x80], &mut out)
            .unwrap();
        assert_eq!(out, [0.0, -1.0]);
    }

    #[test]
    fn pcm24_and_pcm32_keep_sign_and_scale() {
        let mut out = Vec::new();
        format(24, 1, false)
            .decode(&[0, 0, 0x80, 0, 0, 0x40], &mut out)
            .unwrap();
        assert_eq!(out, [-1.0, 0.5]);
        out.clear();
        format(32, 1, false)
            .decode(&[0, 0, 0, 0x80, 0, 0, 0, 0x40], &mut out)
            .unwrap();
        assert_eq!(out, [-1.0, 0.5]);
    }

    #[test]
    fn float_audio_is_finite_and_bounded() {
        let bytes: Vec<_> = [0.25_f32, f32::NAN, f32::INFINITY, 2.0, -2.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut out = Vec::new();
        format(32, 1, true).decode(&bytes, &mut out).unwrap();
        assert_eq!(out, [0.25, 0.0, 0.0, 1.0, -1.0]);
    }

    #[test]
    fn truncated_frames_are_rejected_without_partial_output() {
        let mut out = vec![0.5];
        assert!(format(16, 2, false).decode(&[0, 0, 0], &mut out).is_err());
        assert_eq!(out, [0.5]);
    }

    #[test]
    fn extensible_pcm_accepts_left_aligned_valid_bits() {
        let base = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 22,
            ..Default::default()
        };
        let extension = WAVEFORMATEXTENSIBLE {
            Format: base,
            Samples: windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: 24,
            },
            SubFormat: KSDATAFORMAT_SUBTYPE_PCM,
            ..Default::default()
        };
        assert_eq!(
            MixFormat::parse(base, Some(extension)).unwrap().encoding,
            Encoding::Pcm32
        );
        assert!(MixFormat::parse(base, None).is_err());
        let mut malformed = base;
        malformed.nBlockAlign = 7;
        assert!(MixFormat::parse(malformed, Some(extension)).is_err());
    }
}
