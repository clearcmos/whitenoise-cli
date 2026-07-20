use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

pub const EQ_MIN_DB: f32 = -12.0;
pub const EQ_MAX_DB: f32 = 12.0;

#[derive(Debug, Clone, Copy)]
pub struct FrequencyBand {
    pub name: &'static str,
    pub min_freq: f32,
    pub max_freq: f32,
}

impl FrequencyBand {
    pub fn center_frequency(self) -> f32 {
        (self.min_freq * self.max_freq).sqrt()
    }

    pub fn q(self) -> f32 {
        (self.center_frequency() / (self.max_freq - self.min_freq)).clamp(0.5, 3.0)
    }
}

pub const FREQUENCY_BANDS: [FrequencyBand; 8] = [
    FrequencyBand {
        name: "Sub Bass",
        min_freq: 20.0,
        max_freq: 60.0,
    },
    FrequencyBand {
        name: "Bass",
        min_freq: 60.0,
        max_freq: 250.0,
    },
    FrequencyBand {
        name: "Low Mid",
        min_freq: 250.0,
        max_freq: 500.0,
    },
    FrequencyBand {
        name: "Mid",
        min_freq: 500.0,
        max_freq: 2_000.0,
    },
    FrequencyBand {
        name: "High Mid",
        min_freq: 2_000.0,
        max_freq: 4_000.0,
    },
    FrequencyBand {
        name: "Presence",
        min_freq: 4_000.0,
        max_freq: 6_000.0,
    },
    FrequencyBand {
        name: "Brilliance",
        min_freq: 6_000.0,
        max_freq: 12_000.0,
    },
    FrequencyBand {
        name: "Air",
        min_freq: 12_000.0,
        max_freq: 20_000.0,
    },
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
pub enum SoundStyle {
    #[default]
    #[serde(
        rename = "white",
        alias = "White",
        alias = "Vanilla",
        alias = "vanilla"
    )]
    #[value(name = "white", alias = "vanilla")]
    White,
    #[serde(rename = "pink", alias = "Pink")]
    Pink,
    #[serde(rename = "brown", alias = "Brown")]
    Brown,
    #[serde(rename = "rain", alias = "Rain")]
    Rain,
}

impl SoundStyle {
    pub const ALL: [Self; 4] = [Self::White, Self::Pink, Self::Brown, Self::Rain];

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "White Noise",
            Self::Pink => "Pink Noise",
            Self::Brown => "Brown Noise",
            Self::Rain => "Rain",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::White => Self::Pink,
            Self::Pink => Self::Brown,
            Self::Brown => Self::Rain,
            Self::Rain => Self::White,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub volume: f32,
    pub frequency_bands: [f32; FREQUENCY_BANDS.len()],
    #[serde(alias = "perceptual_normalization")]
    pub listening_contour: bool,
    pub sound_style: SoundStyle,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            // Interactive mode deliberately starts muted unless --volume is supplied.
            volume: 0.0,
            // The middle position is a neutral 0 dB graphic EQ.
            frequency_bands: [0.5; FREQUENCY_BANDS.len()],
            listening_contour: false,
            sound_style: SoundStyle::White,
        }
    }
}

impl AudioSettings {
    pub fn sanitize(mut self) -> Self {
        self.volume = sanitize_unit(self.volume, 0.0);
        for value in &mut self.frequency_bands {
            *value = sanitize_unit(*value, 0.5);
        }
        self
    }
}

pub fn slider_to_db(value: f32) -> f32 {
    EQ_MIN_DB + sanitize_unit(value, 0.5) * (EQ_MAX_DB - EQ_MIN_DB)
}

fn sanitize_unit(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

pub fn config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("whitenoise");
    path.push("settings.toml");
    path
}

pub fn load_settings() -> Result<AudioSettings> {
    load_settings_from(&config_path())
}

fn load_settings_from(path: &std::path::Path) -> Result<AudioSettings> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(AudioSettings::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };

    toml::from_str::<AudioSettings>(&content)
        .with_context(|| format!("failed to parse {}", path.display()))
        .map(AudioSettings::sanitize)
}

pub fn save_settings(settings: &AudioSettings) -> Result<()> {
    save_settings_to(&config_path(), settings)
}

fn save_settings_to(path: &std::path::Path, settings: &AudioSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let content = toml::to_string_pretty(&settings.sanitize())?;
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_slider_is_zero_db() {
        assert_eq!(slider_to_db(0.5), 0.0);
        assert_eq!(slider_to_db(0.0), EQ_MIN_DB);
        assert_eq!(slider_to_db(1.0), EQ_MAX_DB);
    }

    #[test]
    fn legacy_settings_are_migrated() {
        let settings: AudioSettings = toml::from_str(
            r#"
                volume = 0.4
                frequency_bands = [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5]
                perceptual_normalization = true
                sound_style = "Vanilla"
            "#,
        )
        .unwrap();

        assert_eq!(settings.volume, 0.4);
        assert!(settings.listening_contour);
        assert_eq!(settings.sound_style, SoundStyle::White);
    }

    #[test]
    fn missing_fields_receive_safe_defaults() {
        let settings: AudioSettings = toml::from_str("sound_style = \"Rain\"").unwrap();

        assert_eq!(settings.volume, 0.0);
        assert_eq!(settings.frequency_bands, [0.5; FREQUENCY_BANDS.len()]);
        assert_eq!(settings.sound_style, SoundStyle::Rain);
    }

    fn scratch_settings_path(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "whitenoise-settings-test-{}-{label}",
            std::process::id()
        ));
        path.push("nested"); // exercises parent-directory creation on save
        path.push("settings.toml");
        path
    }

    #[test]
    fn settings_survive_a_save_and_load_round_trip() {
        let path = scratch_settings_path("round-trip");
        let saved = AudioSettings {
            volume: 0.35,
            frequency_bands: [0.0, 0.1, 0.2, 0.3, 0.6, 0.7, 0.8, 1.0],
            listening_contour: true,
            sound_style: SoundStyle::Brown,
        };

        save_settings_to(&path, &saved).unwrap();
        let loaded = load_settings_from(&path).unwrap();
        assert_eq!(loaded, saved);

        std::fs::remove_dir_all(path.ancestors().nth(2).unwrap()).unwrap();
    }

    #[test]
    fn missing_settings_file_yields_defaults() {
        let path = scratch_settings_path("missing");
        let loaded = load_settings_from(&path).unwrap();
        assert_eq!(loaded, AudioSettings::default());
    }

    #[test]
    fn malformed_settings_file_reports_the_path() {
        let path = scratch_settings_path("malformed");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "volume = \"not a number\"").unwrap();

        let error = format!("{:#}", load_settings_from(&path).unwrap_err());
        assert!(
            error.contains("failed to parse"),
            "unexpected error: {error}"
        );
        assert!(error.contains("settings.toml"));

        std::fs::remove_dir_all(path.ancestors().nth(2).unwrap()).unwrap();
    }

    #[test]
    fn out_of_range_values_are_sanitized_on_save() {
        let path = scratch_settings_path("sanitize-on-save");
        let saved = AudioSettings {
            volume: 7.0,
            ..AudioSettings::default()
        };

        save_settings_to(&path, &saved).unwrap();
        let loaded = load_settings_from(&path).unwrap();
        assert_eq!(loaded.volume, 1.0);

        std::fs::remove_dir_all(path.ancestors().nth(2).unwrap()).unwrap();
    }

    #[test]
    fn every_style_round_trips_through_toml() {
        for style in SoundStyle::ALL {
            let saved = toml::to_string(&AudioSettings {
                sound_style: style,
                ..AudioSettings::default()
            })
            .unwrap();
            let loaded: AudioSettings = toml::from_str(&saved).unwrap();
            assert_eq!(loaded.sound_style, style);
        }
    }

    #[test]
    fn style_cycle_visits_every_style_once() {
        let mut style = SoundStyle::White;
        let mut visited = Vec::new();
        for _ in 0..SoundStyle::ALL.len() {
            visited.push(style);
            style = style.next();
        }
        assert_eq!(visited, SoundStyle::ALL);
        assert_eq!(style, SoundStyle::White);
    }

    #[test]
    fn invalid_numeric_values_are_sanitized() {
        let settings = AudioSettings {
            volume: f32::NAN,
            frequency_bands: [2.0, -1.0, 0.5, 0.5, 0.5, 0.5, 0.5, f32::INFINITY],
            ..AudioSettings::default()
        }
        .sanitize();

        assert_eq!(settings.volume, 0.0);
        assert_eq!(settings.frequency_bands[0], 1.0);
        assert_eq!(settings.frequency_bands[1], 0.0);
        assert_eq!(settings.frequency_bands[7], 0.5);
    }
}
