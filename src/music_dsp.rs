//! Bounded, local audio analysis and keyboard color generation.

use std::f32::consts::TAU;

pub const BANDS: usize = 16;
const WINDOW: usize = 2048;
const MATRICES: usize = 126;

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioFrame {
    pub bands: [f32; BANDS],
    pub level: f32,
    pub beat: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Spectrum,
    Pulse,
    Wave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Neon,
    Rainbow,
    Ocean,
    Sunset,
    Lagoon,
    Glacier,
    Abyss,
    Aurora,
    SeaGlass,
    BlueHour,
    CoralReef,
    MidnightTide,
    GoldenHour,
    PeachGlow,
    RoseDusk,
    Ember,
    DesertBloom,
    ApricotSky,
    VelvetDusk,
    TropicalDusk,
}

impl Palette {
    pub const ALL: [Self; 20] = [
        Self::Ocean,
        Self::Lagoon,
        Self::Glacier,
        Self::Abyss,
        Self::Aurora,
        Self::SeaGlass,
        Self::BlueHour,
        Self::CoralReef,
        Self::MidnightTide,
        Self::Sunset,
        Self::GoldenHour,
        Self::PeachGlow,
        Self::RoseDusk,
        Self::Ember,
        Self::DesertBloom,
        Self::ApricotSky,
        Self::VelvetDusk,
        Self::TropicalDusk,
        Self::Neon,
        Self::Rainbow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ocean => "Ocean",
            Self::Lagoon => "Lagoon",
            Self::Glacier => "Glacier",
            Self::Abyss => "Abyss",
            Self::Aurora => "Aurora",
            Self::SeaGlass => "Sea glass",
            Self::BlueHour => "Blue hour",
            Self::CoralReef => "Coral reef",
            Self::MidnightTide => "Midnight tide",
            Self::Sunset => "Sunset",
            Self::GoldenHour => "Golden hour",
            Self::PeachGlow => "Peach glow",
            Self::RoseDusk => "Rose dusk",
            Self::Ember => "Ember",
            Self::DesertBloom => "Desert bloom",
            Self::ApricotSky => "Apricot sky",
            Self::VelvetDusk => "Velvet dusk",
            Self::TropicalDusk => "Tropical dusk",
            Self::Neon => "Neon",
            Self::Rainbow => "Rainbow",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Ocean => "Blue / turquoise / seafoam",
            Self::Lagoon => "Teal / jade / mint",
            Self::Glacier => "Ice blue / cyan / pearl",
            Self::Abyss => "Navy / cobalt / violet",
            Self::Aurora => "Emerald / cyan / lavender",
            Self::SeaGlass => "Aqua / sage / soft gold",
            Self::BlueHour => "Indigo / periwinkle / rose",
            Self::CoralReef => "Turquoise / coral / peach",
            Self::MidnightTide => "Deep blue / teal / silver",
            Self::Sunset => "Coral / amber / hot pink",
            Self::GoldenHour => "Amber / honey / champagne",
            Self::PeachGlow => "Rose / peach / cream",
            Self::RoseDusk => "Berry / rose / blush",
            Self::Ember => "Crimson / flame / gold",
            Self::DesertBloom => "Terracotta / sand / orchid",
            Self::ApricotSky => "Apricot / blush / lilac",
            Self::VelvetDusk => "Plum / magenta / tangerine",
            Self::TropicalDusk => "Coral / pink / violet / aqua",
            Self::Neon => "Green / cyan / violet / pink",
            Self::Rainbow => "The full color spectrum",
        }
    }

    pub fn family(self) -> &'static str {
        match self {
            Self::Ocean
            | Self::Lagoon
            | Self::Glacier
            | Self::Abyss
            | Self::Aurora
            | Self::SeaGlass
            | Self::BlueHour
            | Self::CoralReef
            | Self::MidnightTide => "Ocean",
            Self::Sunset
            | Self::GoldenHour
            | Self::PeachGlow
            | Self::RoseDusk
            | Self::Ember
            | Self::DesertBloom
            | Self::ApricotSky
            | Self::VelvetDusk
            | Self::TropicalDusk => "Sunset",
            Self::Neon | Self::Rainbow => "Studio",
        }
    }

    /// Unscaled RGB channels in 0..=255, shared by swatches and live lighting.
    /// Positions outside the gradient are clamped; non-finite input starts at 0.
    pub fn color(self, position: f32) -> [f32; 3] {
        let position = unit(position);
        let stops = match self {
            Self::Neon => [
                [35.0, 255.0, 110.0],
                [8.0, 224.0, 214.0],
                [91.0, 86.0, 255.0],
                [249.0, 50.0, 172.0],
            ],
            Self::Ocean => [
                [13.0, 93.0, 245.0],
                [9.0, 166.0, 255.0],
                [15.0, 232.0, 218.0],
                [134.0, 255.0, 238.0],
            ],
            Self::Sunset => [
                [255.0, 71.0, 82.0],
                [255.0, 124.0, 48.0],
                [249.0, 193.0, 73.0],
                [240.0, 61.0, 163.0],
            ],
            Self::Lagoon => [
                [8.0, 112.0, 134.0],
                [12.0, 194.0, 178.0],
                [58.0, 231.0, 157.0],
                [178.0, 255.0, 208.0],
            ],
            Self::Glacier => [
                [61.0, 115.0, 238.0],
                [91.0, 194.0, 255.0],
                [160.0, 241.0, 255.0],
                [235.0, 250.0, 255.0],
            ],
            Self::Abyss => [
                [16.0, 28.0, 100.0],
                [31.0, 62.0, 203.0],
                [84.0, 71.0, 244.0],
                [155.0, 101.0, 248.0],
            ],
            Self::Aurora => [
                [30.0, 224.0, 126.0],
                [37.0, 226.0, 215.0],
                [103.0, 156.0, 255.0],
                [204.0, 133.0, 255.0],
            ],
            Self::SeaGlass => [
                [76.0, 180.0, 177.0],
                [137.0, 215.0, 194.0],
                [187.0, 219.0, 166.0],
                [242.0, 226.0, 173.0],
            ],
            Self::BlueHour => [
                [49.0, 53.0, 158.0],
                [103.0, 112.0, 228.0],
                [174.0, 158.0, 246.0],
                [239.0, 168.0, 213.0],
            ],
            Self::CoralReef => [
                [12.0, 169.0, 190.0],
                [75.0, 231.0, 205.0],
                [255.0, 123.0, 136.0],
                [255.0, 193.0, 143.0],
            ],
            Self::MidnightTide => [
                [23.0, 45.0, 111.0],
                [25.0, 116.0, 155.0],
                [76.0, 179.0, 183.0],
                [191.0, 216.0, 224.0],
            ],
            Self::GoldenHour => [
                [244.0, 116.0, 32.0],
                [255.0, 172.0, 46.0],
                [255.0, 213.0, 99.0],
                [255.0, 239.0, 187.0],
            ],
            Self::PeachGlow => [
                [242.0, 116.0, 151.0],
                [255.0, 158.0, 146.0],
                [255.0, 197.0, 164.0],
                [255.0, 234.0, 201.0],
            ],
            Self::RoseDusk => [
                [137.0, 45.0, 120.0],
                [202.0, 73.0, 143.0],
                [241.0, 122.0, 166.0],
                [255.0, 193.0, 198.0],
            ],
            Self::Ember => [
                [175.0, 25.0, 53.0],
                [245.0, 60.0, 31.0],
                [255.0, 122.0, 25.0],
                [255.0, 204.0, 66.0],
            ],
            Self::DesertBloom => [
                [192.0, 88.0, 66.0],
                [232.0, 155.0, 106.0],
                [245.0, 201.0, 151.0],
                [204.0, 126.0, 188.0],
            ],
            Self::ApricotSky => [
                [255.0, 156.0, 103.0],
                [255.0, 190.0, 152.0],
                [242.0, 172.0, 203.0],
                [181.0, 153.0, 242.0],
            ],
            Self::VelvetDusk => [
                [92.0, 34.0, 151.0],
                [169.0, 47.0, 169.0],
                [235.0, 78.0, 132.0],
                [255.0, 149.0, 76.0],
            ],
            Self::TropicalDusk => [
                [255.0, 116.0, 91.0],
                [241.0, 83.0, 170.0],
                [136.0, 100.0, 242.0],
                [52.0, 217.0, 212.0],
            ],
            Self::Rainbow => {
                let hue = position * 5.999;
                let segment = hue.floor() as usize;
                let up = hue.fract() * 255.0;
                let down = 255.0 - up;
                return match segment {
                    0 => [255.0, up, 0.0],
                    1 => [down, 255.0, 0.0],
                    2 => [0.0, 255.0, up],
                    3 => [0.0, down, 255.0],
                    4 => [up, 0.0, 255.0],
                    _ => [255.0, 0.0, down],
                };
            }
        };
        let position = position * (stops.len() - 1) as f32;
        let index = position.floor() as usize;
        std::array::from_fn(|channel| {
            lerp(
                stops[index][channel],
                stops[(index + 1).min(stops.len() - 1)][channel],
                position.fract(),
            )
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub gain: f32,
    pub smoothing: f32,
    pub brightness: f32,
    pub mode: Mode,
    pub palette: Palette,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gain: 2.0,
            smoothing: 0.65,
            brightness: 1.0,
            mode: Mode::Spectrum,
            palette: Palette::Neon,
        }
    }
}

impl Settings {
    fn bounded(self) -> Self {
        Self {
            gain: finite(self.gain, 2.0).clamp(0.05, 8.0),
            smoothing: finite(self.smoothing, 0.65).clamp(0.0, 0.98),
            brightness: unit(self.brightness),
            ..self
        }
    }
}

pub struct Analyzer {
    sample_rate: f32,
    samples: [f32; WINDOW],
    cursor: usize,
    window: [f32; WINDOW],
    window_sum: f32,
    frame: AudioFrame,
    previous_energy: f32,
    average_energy: f32,
    refractory: f32,
}

impl Analyzer {
    pub fn new(sample_rate: u32) -> Self {
        let window =
            std::array::from_fn(|i| 0.5 - 0.5 * (TAU * i as f32 / (WINDOW - 1) as f32).cos());
        Self {
            sample_rate: if sample_rate == 0 {
                48_000.0
            } else {
                sample_rate.clamp(8_000, 384_000) as f32
            },
            samples: [0.0; WINDOW],
            cursor: 0,
            window_sum: window.iter().sum(),
            window,
            frame: AudioFrame::default(),
            previous_energy: 0.0,
            average_energy: 0.0,
            refractory: 0.0,
        }
    }

    /// Analyze mono PCM. An empty update advances silence by one 60 Hz frame.
    /// Only the most recent 2048 samples are retained or transformed.
    pub fn update(&mut self, samples: &[f32], settings: Settings) -> AudioFrame {
        let settings = settings.bounded();
        let elapsed = if samples.is_empty() {
            1.0 / 60.0
        } else {
            (samples.len() as f32 / self.sample_rate).clamp(0.001, 0.25)
        };
        if samples.is_empty() {
            let zeros = (self.sample_rate / 60.0).ceil() as usize;
            for _ in 0..zeros.min(WINDOW) {
                self.push(0.0);
            }
        } else {
            for &sample in &samples[samples.len().saturating_sub(WINDOW)..] {
                self.push(finite(sample, 0.0).clamp(-1.0, 1.0));
            }
        }

        let mut real = [0.0; WINDOW];
        let mut imaginary = [0.0; WINDOW];
        let mut square_sum = 0.0;
        for (i, value) in real.iter_mut().enumerate() {
            let sample = self.samples[(self.cursor + i) % WINDOW];
            square_sum += sample * sample;
            *value = sample * self.window[i];
        }
        let rms = (square_sum / WINDOW as f32).sqrt();
        fft(&mut real, &mut imaginary);

        let mut band_energy = [0.0; BANDS];
        let low = 40.0_f32;
        let high = 16_000.0_f32.min(self.sample_rate * 0.475);
        let log_range = (high / low).ln();
        if rms >= 0.000_5 {
            for bin in 1..WINDOW / 2 {
                let frequency = bin as f32 * self.sample_rate / WINDOW as f32;
                if frequency < low || frequency >= high {
                    continue;
                }
                let band = (((frequency / low).ln() / log_range) * BANDS as f32).floor() as usize;
                let magnitude = real[bin].hypot(imaginary[bin]) * 2.0 / self.window_sum;
                band_energy[band.min(BANDS - 1)] += magnitude * magnitude;
            }
        }

        let attack = 0.012 + settings.smoothing * 0.018;
        let release = 0.04 + settings.smoothing * 0.32;
        for (envelope, energy) in self.frame.bands.iter_mut().zip(band_energy) {
            let target = (energy.sqrt() * settings.gain * 1.6).sqrt().min(1.0);
            *envelope = follow(*envelope, target, elapsed, attack, release);
        }
        let target_level = (rms * settings.gain * 2.0).min(1.0);
        self.frame.level = follow(self.frame.level, target_level, elapsed, attack, release);

        // Positive energy change detects musical attacks without repeatedly
        // classifying a sustained tone as a new beat.
        let bass = band_energy[..5].iter().sum::<f32>().sqrt();
        let energy = bass * 0.7 + rms * 0.3;
        let novelty = (energy - self.previous_energy).max(0.0);
        let threshold = 0.008 + self.average_energy * 0.18;
        self.refractory = (self.refractory - elapsed).max(0.0);
        self.frame.beat *= (-elapsed / 0.14).exp();
        if novelty > threshold && energy > 0.015 && self.refractory == 0.0 {
            self.frame.beat = (novelty / (threshold * 3.0)).clamp(0.3, 1.0);
            self.refractory = 0.11;
        }
        self.average_energy += (energy - self.average_energy) * (1.0 - (-elapsed / 0.4).exp());
        self.previous_energy = energy;
        self.frame
    }

    fn push(&mut self, sample: f32) {
        self.samples[self.cursor] = sample;
        self.cursor = (self.cursor + 1) % WINDOW;
    }
}

fn fft(real: &mut [f32; WINDOW], imaginary: &mut [f32; WINDOW]) {
    for i in 0..WINDOW {
        let j = i.reverse_bits() >> (usize::BITS - WINDOW.ilog2());
        if i < j {
            real.swap(i, j);
            imaginary.swap(i, j);
        }
    }
    let mut width = 2;
    while width <= WINDOW {
        let (step_imaginary, step_real) = (-TAU / width as f32).sin_cos();
        for start in (0..WINDOW).step_by(width) {
            let mut twiddle_real = 1.0;
            let mut twiddle_imaginary = 0.0;
            for offset in 0..width / 2 {
                let a = start + offset;
                let b = a + width / 2;
                let product_real = twiddle_real * real[b] - twiddle_imaginary * imaginary[b];
                let product_imaginary = twiddle_real * imaginary[b] + twiddle_imaginary * real[b];
                real[b] = real[a] - product_real;
                imaginary[b] = imaginary[a] - product_imaginary;
                real[a] += product_real;
                imaginary[a] += product_imaginary;
                let next_real = twiddle_real * step_real - twiddle_imaginary * step_imaginary;
                twiddle_imaginary = twiddle_real * step_imaginary + twiddle_imaginary * step_real;
                twiddle_real = next_real;
            }
        }
        width *= 2;
    }
}

pub fn colors(
    frame: AudioFrame,
    settings: Settings,
    keys: &[(usize, f32, f32)],
    phase: f32,
) -> [[u8; 3]; MATRICES] {
    let settings = settings.bounded();
    let phase = finite(phase, 0.0).rem_euclid(TAU * 100.0);
    let bands = frame.bands.map(unit);
    let level = unit(frame.level);
    let beat = unit(frame.beat);
    let bass = bands[..5].iter().sum::<f32>() / 5.0;
    let mut output = [[0; 3]; MATRICES];
    for &(matrix, x, y) in keys {
        if matrix >= MATRICES || !x.is_finite() || !y.is_finite() {
            continue;
        }
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        let (intensity, hue) = match settings.mode {
            Mode::Spectrum => {
                let position = x * (BANDS - 1) as f32;
                let index = position.floor() as usize;
                let amplitude = lerp(
                    bands[index],
                    bands[(index + 1).min(BANDS - 1)],
                    position.fract(),
                );
                let fill = if amplitude <= 0.015 {
                    0.0
                } else {
                    smoothstep((amplitude - (1.0 - y) + 0.06) / 0.12)
                };
                (fill * (0.18 + 0.82 * amplitude), x)
            }
            Mode::Pulse => (
                (level * 0.8 + bass * 0.25 + beat * 0.8).min(1.0),
                (x * 0.6 + beat * 0.25).min(1.0),
            ),
            Mode::Wave => {
                let wave = ((phase * 1.8 - (x * 0.85 + y * 0.3) * TAU).sin() + 1.0) * 0.5;
                (
                    (level * 0.65 + beat * 0.75).min(1.0) * (0.18 + wave * 0.82),
                    (x + phase * 0.07).rem_euclid(1.0),
                )
            }
        };
        let color = settings.palette.color(hue);
        let intensity = unit(intensity) * settings.brightness;
        output[matrix] = color.map(|channel| (channel * intensity).round() as u8);
    }
    output
}

fn follow(value: f32, target: f32, elapsed: f32, attack: f32, release: f32) -> f32 {
    let tau = if target > value { attack } else { release };
    let value = lerp(value, target, 1.0 - (-elapsed / tau).exp());
    if value < 0.000_01 { 0.0 } else { unit(value) }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn unit(value: f32) -> f32 {
    finite(value, 0.0).clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, amount: f32) -> f32 {
    a + (b - a) * amount
}

fn smoothstep(value: f32) -> f32 {
    let value = unit(value);
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_twenty_named_and_visually_distinct_palettes() {
        let mut names = Vec::new();
        let mut variants = Vec::new();
        let mut gradients = Vec::new();
        for palette in Palette::ALL {
            assert!(!names.contains(&palette.name()), "{}", palette.name());
            assert!(!variants.contains(&palette), "{palette:?}");
            assert!(!palette.description().is_empty());
            let gradient: Vec<_> = (0..=32)
                .map(|step| palette.color(step as f32 / 32.0).map(|v| v.round() as u8))
                .collect();
            assert!(!gradients.contains(&gradient), "{}", palette.name());
            names.push(palette.name());
            variants.push(palette);
            gradients.push(gradient);
        }
        assert_eq!(names.len(), 20);
        let families: Vec<_> = Palette::ALL.into_iter().map(Palette::family).collect();
        assert_eq!(&families[..9], &["Ocean"; 9]);
        assert_eq!(&families[9..18], &["Sunset"; 9]);
        assert_eq!(&families[18..], &["Studio"; 2]);
    }

    #[test]
    fn palette_interpolation_is_continuous_bounded_and_handles_invalid_positions() {
        for palette in Palette::ALL {
            let mut previous = palette.color(0.0);
            for step in 0..=1024 {
                let color = palette.color(step as f32 / 1024.0);
                for (channel, previous_channel) in color.into_iter().zip(previous) {
                    assert!(channel.is_finite() && (0.0..=255.0).contains(&channel));
                    assert!((channel - previous_channel).abs() <= 2.0, "{palette:?}");
                }
                previous = color;
            }
            for position in [-1.0, -1e30, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(palette.color(position), palette.color(0.0), "{palette:?}");
            }
            for position in [1.1, 1e30] {
                assert_eq!(palette.color(position), palette.color(1.0), "{palette:?}");
            }
        }
    }

    #[test]
    fn original_palette_colors_are_preserved() {
        let fixtures = [
            (
                Palette::Neon,
                [
                    [35.0, 255.0, 110.0],
                    [14.75, 231.75, 188.0],
                    [49.5, 155.0, 234.5],
                    [130.5, 77.0, 234.25],
                    [249.0, 50.0, 172.0],
                ],
            ),
            (
                Palette::Ocean,
                [
                    [13.0, 93.0, 245.0],
                    [10.0, 147.75, 252.5],
                    [12.0, 199.0, 236.5],
                    [44.75, 237.75, 223.0],
                    [134.0, 255.0, 238.0],
                ],
            ),
            (
                Palette::Sunset,
                [
                    [255.0, 71.0, 82.0],
                    [255.0, 110.75, 56.5],
                    [252.0, 158.5, 60.5],
                    [246.75, 160.0, 95.5],
                    [240.0, 61.0, 163.0],
                ],
            ),
            (
                Palette::Rainbow,
                [
                    [255.0, 0.0, 0.0],
                    [127.563_75, 255.0, 0.0],
                    [0.0, 255.0, 254.872_5],
                    [127.308_75, 0.0, 255.0],
                    [255.0, 0.0, 0.255],
                ],
            ),
        ];
        for (palette, expected_colors) in fixtures {
            for (step, expected) in expected_colors.into_iter().enumerate() {
                let actual = palette.color(step as f32 / 4.0);
                for (actual, expected) in actual.into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 0.002,
                        "{palette:?} at step {step}: {actual} != {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn full_spectrum_uses_the_same_gradient_as_the_palette_preview() {
        let keys: Vec<_> = (0..=32)
            .map(|matrix| (matrix, matrix as f32 / 32.0, 1.0))
            .collect();
        let frame = AudioFrame {
            bands: [1.0; BANDS],
            level: 1.0,
            beat: 0.0,
        };
        for palette in Palette::ALL {
            let settings = Settings {
                palette,
                ..Settings::default()
            };
            let output = colors(frame, settings, &keys, 0.0);
            for &(matrix, x, _) in &keys {
                assert_eq!(
                    output[matrix],
                    palette.color(x).map(|channel| channel.round() as u8),
                    "{palette:?} at key {matrix}"
                );
            }
        }
    }

    fn tone(frequency: f32, amplitude: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| (TAU * frequency * i as f32 / 48_000.0).sin() * amplitude)
            .collect()
    }

    fn dominant(frame: AudioFrame) -> usize {
        frame
            .bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0
    }

    #[test]
    fn spectrum_separates_bass_mid_and_treble_sine_waves() {
        let settings = Settings {
            smoothing: 0.0,
            ..Settings::default()
        };
        let frames = [94.0, 750.0, 6000.0].map(|frequency| {
            Analyzer::new(48_000).update(&tone(frequency, 0.15, WINDOW), settings)
        });
        let bands = frames.map(dominant);
        assert!(bands[0] < bands[1] && bands[1] < bands[2], "{bands:?}");
        assert!(bands[1] - bands[0] >= 4 && bands[2] - bands[1] >= 4);
        for frame in frames {
            assert!(frame.bands[dominant(frame)] > 0.5);
            assert!(frame.level > 0.3);
        }
    }

    #[test]
    fn silence_is_black_and_missing_audio_decays_to_silence() {
        let settings = Settings::default();
        let keys = [(3, 0.1, 1.0), (125, 0.9, 0.2)];
        let mut analyzer = Analyzer::new(48_000);
        let quiet = analyzer.update(&[], settings);
        assert_eq!(colors(quiet, settings, &keys, 1.0), [[0; 3]; MATRICES]);
        let loud = analyzer.update(&tone(94.0, 0.3, WINDOW), settings);
        assert!(loud.level > 0.1 && loud.beat > 0.0);
        let mut quiet = loud;
        for _ in 0..300 {
            quiet = analyzer.update(&[], settings);
        }
        assert_eq!(quiet.bands, [0.0; BANDS]);
        assert_eq!(quiet.level, 0.0);
        assert!(quiet.beat < 0.000_01);
        for mode in [Mode::Spectrum, Mode::Pulse, Mode::Wave] {
            for palette in Palette::ALL {
                let settings = Settings {
                    mode,
                    palette,
                    ..settings
                };
                assert_eq!(colors(quiet, settings, &keys, 1.0), [[0; 3]; MATRICES]);
            }
        }
    }

    #[test]
    fn sustained_tone_does_not_retrigger_beats_and_a_new_attack_does() {
        let mut analyzer = Analyzer::new(48_000);
        let settings = Settings::default();
        let tone = tone(94.0, 0.3, WINDOW);
        let attack = analyzer.update(&tone, settings);
        assert!(attack.beat >= 0.3);
        let mut sustained = attack;
        for _ in 0..30 {
            sustained = analyzer.update(&tone, settings);
        }
        assert!(sustained.beat < 0.001, "{sustained:?}");
        for _ in 0..12 {
            analyzer.update(&[], settings);
        }
        let second_attack = analyzer.update(&tone, settings);
        assert!(second_attack.beat >= 0.3);
        assert!(second_attack.level > 0.0);
    }

    #[test]
    fn oversized_input_uses_only_the_recent_window() {
        let settings = Settings::default();
        let tail = tone(750.0, 0.15, WINDOW);
        let mut long = tone(94.0, 0.9, WINDOW * 8);
        long.extend_from_slice(&tail);
        let frame = Analyzer::new(48_000).update(&long, settings);
        assert_eq!(
            dominant(frame),
            dominant(Analyzer::new(48_000).update(&tail, settings))
        );
        assert!(dominant(frame) >= 6);
    }

    #[test]
    fn malformed_samples_and_settings_remain_bounded() {
        let settings = Settings {
            gain: f32::INFINITY,
            smoothing: f32::NAN,
            brightness: -1.0,
            mode: Mode::Wave,
            palette: Palette::Rainbow,
        };
        let mut analyzer = Analyzer::new(0);
        let frame = analyzer.update(
            &[f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30],
            settings,
        );
        for value in frame.bands.into_iter().chain([frame.level, frame.beat]) {
            assert!(value.is_finite() && (0.0..=1.0).contains(&value));
        }
        assert_eq!(
            colors(frame, settings, &[(0, 0.5, 0.5)], f32::NAN),
            [[0; 3]; MATRICES]
        );
    }

    #[test]
    fn effects_write_only_valid_provided_keys_and_obey_brightness() {
        let frame = AudioFrame {
            bands: [0.8; BANDS],
            level: 0.7,
            beat: 0.9,
        };
        let keys = [
            (0, 0.0, 1.0),
            (125, 1.0, 1.0),
            (126, 0.5, 1.0),
            (usize::MAX, 0.5, 1.0),
            (22, f32::NAN, 1.0),
        ];
        for mode in [Mode::Spectrum, Mode::Pulse, Mode::Wave] {
            for palette in Palette::ALL {
                let settings = Settings {
                    mode,
                    palette,
                    ..Settings::default()
                };
                let bright = colors(frame, settings, &keys, 0.5);
                assert!(bright[0].iter().any(|&channel| channel > 0));
                assert!(bright[125].iter().any(|&channel| channel > 0));
                assert!(bright[1..125].iter().all(|&color| color == [0; 3]));
                let dim = colors(
                    frame,
                    Settings {
                        brightness: 0.25,
                        ..settings
                    },
                    &keys,
                    0.5,
                );
                for (full, dim) in bright.into_iter().flatten().zip(dim.into_iter().flatten()) {
                    assert!(dim <= full);
                }
            }
        }
    }

    #[test]
    fn spectrum_fills_from_bottom_and_wave_moves_with_time() {
        let frame = AudioFrame {
            bands: [0.4; BANDS],
            level: 0.5,
            beat: 0.2,
        };
        let keys = [(0, 0.2, 0.0), (1, 0.2, 1.0)];
        let spectrum = colors(frame, Settings::default(), &keys, 0.0);
        assert_eq!(spectrum[0], [0; 3]);
        assert_ne!(spectrum[1], [0; 3]);
        let settings = Settings {
            mode: Mode::Wave,
            ..Settings::default()
        };
        assert_ne!(
            colors(frame, settings, &keys, 0.0),
            colors(frame, settings, &keys, std::f32::consts::PI)
        );
    }
}
