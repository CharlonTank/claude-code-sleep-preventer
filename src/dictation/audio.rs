use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AudioRecorder {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
    channels: u16,
}

impl AudioRecorder {
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No audio input device available")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get input config: {}", e))?;

        Ok(Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        })
    }

    pub fn start_recording(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("No audio input device")?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get config: {}", e))?;

        self.sample_rate = config.sample_rate().0;
        self.channels = config.channels();

        // Clear previous samples
        self.samples.lock().unwrap().clear();
        let samples_clone = self.samples.clone();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _| {
                    samples_clone.lock().unwrap().extend_from_slice(data);
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            ),
            cpal::SampleFormat::I16 => {
                let samples_clone = self.samples.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let floats: Vec<f32> =
                            data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        samples_clone.lock().unwrap().extend_from_slice(&floats);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let samples_clone = self.samples.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _| {
                        let floats: Vec<f32> = data
                            .iter()
                            .map(|&s| (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
                            .collect();
                        samples_clone.lock().unwrap().extend_from_slice(&floats);
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            format => return Err(format!("Unsupported sample format: {:?}", format)),
        }
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        self.stream = Some(stream);
        Ok(())
    }

    pub fn stop_recording(&mut self) -> Vec<f32> {
        // Drop the stream to stop recording
        self.stream = None;
        std::mem::take(&mut *self.samples.lock().unwrap())
    }

    /// Save audio as 16kHz mono WAV (whisper-cpp format)
    pub fn save_to_wav(&self, samples: &[f32], path: &PathBuf) -> Result<(), String> {
        const TARGET_SAMPLE_RATE: u32 = 16000;

        // Convert to mono if stereo
        let mono_samples: Vec<f32> = if self.channels > 1 {
            samples
                .chunks(self.channels as usize)
                .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                .collect()
        } else {
            samples.to_vec()
        };

        // Resample to 16kHz if needed
        let resampled = if self.sample_rate != TARGET_SAMPLE_RATE {
            Self::resample(&mono_samples, self.sample_rate, TARGET_SAMPLE_RATE)
        } else {
            mono_samples
        };

        let spec = WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        let mut writer =
            WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {}", e))?;

        for &sample in &resampled {
            let amplitude = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(amplitude)
                .map_err(|e| format!("Failed to write sample: {}", e))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("Failed to finalize WAV: {}", e))?;

        Ok(())
    }

    /// Linear interpolation resampling
    fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = (samples.len() as f64 / ratio).ceil() as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_pos = i as f64 * ratio;
            let src_idx = src_pos.floor() as usize;
            let frac = (src_pos - src_idx as f64) as f32;

            let sample = if src_idx + 1 < samples.len() {
                // Linear interpolation
                samples[src_idx] * (1.0 - frac) + samples[src_idx + 1] * frac
            } else if src_idx < samples.len() {
                samples[src_idx]
            } else {
                0.0
            };
            output.push(sample);
        }

        output
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn has_samples(&self) -> bool {
        !self.samples.lock().unwrap().is_empty()
    }
}
