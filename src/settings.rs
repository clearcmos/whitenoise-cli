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

/// Per-source playback levels as power fractions in [0, 1]. Levels are
/// independent (they need not sum to 1); the engine takes sqrt(level) as the
/// mixing amplitude, so a 0.5/0.5 mix carries equal power from each source
/// and a solo at 1.0 is bit-identical to the pre-mixing behavior.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceMix {
    pub white: f32,
    pub pink: f32,
    pub brown: f32,
    pub rain: f32,
}

impl Default for SourceMix {
    fn default() -> Self {
        Self::solo(SoundStyle::White)
    }
}

impl SourceMix {
    pub fn solo(style: SoundStyle) -> Self {
        let mut mix = Self {
            white: 0.0,
            pink: 0.0,
            brown: 0.0,
            rain: 0.0,
        };
        mix.set_level(style, 1.0);
        mix
    }

    pub fn level(&self, style: SoundStyle) -> f32 {
        match style {
            SoundStyle::White => self.white,
            SoundStyle::Pink => self.pink,
            SoundStyle::Brown => self.brown,
            SoundStyle::Rain => self.rain,
        }
    }

    pub fn set_level(&mut self, style: SoundStyle, value: f32) {
        let slot = match style {
            SoundStyle::White => &mut self.white,
            SoundStyle::Pink => &mut self.pink,
            SoundStyle::Brown => &mut self.brown,
            SoundStyle::Rain => &mut self.rain,
        };
        *slot = value;
    }

    pub fn total(&self) -> f32 {
        SoundStyle::ALL.iter().map(|style| self.level(*style)).sum()
    }

    /// The single active source, if exactly one level is above zero.
    pub fn solo_style(&self) -> Option<SoundStyle> {
        let mut active = SoundStyle::ALL
            .into_iter()
            .filter(|style| self.level(*style) > 0.0);
        match (active.next(), active.next()) {
            (Some(style), None) => Some(style),
            _ => None,
        }
    }

    /// The loudest source; ties resolve in SoundStyle::ALL order, and an
    /// all-zero mix reports White so style cycling always has an anchor.
    pub fn dominant(&self) -> SoundStyle {
        SoundStyle::ALL
            .into_iter()
            .rev()
            .max_by(|a, b| self.level(*a).total_cmp(&self.level(*b)))
            .unwrap_or(SoundStyle::White)
    }

    pub fn describe(&self) -> String {
        if let Some(style) = self.solo_style() {
            return style.label().to_owned();
        }
        if self.total() <= 0.0 {
            return "Silence (all sources at zero)".to_owned();
        }
        let parts: Vec<String> = SoundStyle::ALL
            .into_iter()
            .filter(|style| self.level(*style) > 0.0)
            .map(|style| format!("{} {:.0}%", style.label(), self.level(style) * 100.0))
            .collect();
        format!("Mix: {}", parts.join(" + "))
    }

    fn sanitize(mut self) -> Self {
        for style in SoundStyle::ALL {
            self.set_level(style, sanitize_unit(self.level(style), 0.0));
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub volume: f32,
    pub frequency_bands: [f32; FREQUENCY_BANDS.len()],
    #[serde(alias = "perceptual_normalization")]
    pub listening_contour: bool,
    // Kept in the file as the dominant source so pre-mix binaries can still
    // read new settings; at runtime it only anchors legacy migration.
    pub sound_style: SoundStyle,
    // Access through mix()/set_mix(); pub(crate) only so struct-update
    // syntax keeps working in the other modules' tests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mix: Option<SourceMix>,
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
            mix: None,
        }
    }
}

impl AudioSettings {
    pub fn sanitize(mut self) -> Self {
        self.volume = sanitize_unit(self.volume, 0.0);
        for value in &mut self.frequency_bands {
            *value = sanitize_unit(*value, 0.5);
        }
        self.mix = Some(self.mix().sanitize());
        self
    }

    /// The effective mix. A settings file predating the [mix] table migrates
    /// to a solo of its legacy sound_style.
    pub fn mix(&self) -> SourceMix {
        self.mix
            .unwrap_or_else(|| SourceMix::solo(self.sound_style))
    }

    pub fn set_mix(&mut self, mix: SourceMix) {
        self.mix = Some(mix.sanitize());
        self.sound_style = self.mix().dominant();
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
        let mut saved = AudioSettings {
            volume: 0.35,
            frequency_bands: [0.0, 0.1, 0.2, 0.3, 0.6, 0.7, 0.8, 1.0],
            listening_contour: true,
            ..AudioSettings::default()
        };
        saved.set_mix(SourceMix {
            white: 0.0,
            pink: 0.25,
            brown: 0.5,
            rain: 0.0,
        });

        save_settings_to(&path, &saved).unwrap();
        let loaded = load_settings_from(&path).unwrap();
        assert_eq!(loaded, saved.sanitize());
        assert_eq!(loaded.mix().brown, 0.5);
        // The dominant source is persisted as sound_style so binaries that
        // predate the [mix] table still play something sensible.
        assert_eq!(loaded.sound_style, SoundStyle::Brown);

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
    fn legacy_files_without_a_mix_table_migrate_to_a_solo() {
        // Files written before source mixing existed carry only sound_style.
        let settings: AudioSettings = toml::from_str("sound_style = \"pink\"").unwrap();
        assert_eq!(settings.mix(), SourceMix::solo(SoundStyle::Pink));
        assert_eq!(settings.mix().solo_style(), Some(SoundStyle::Pink));
    }

    #[test]
    fn mix_solo_and_dominant_semantics() {
        let mix = SourceMix {
            white: 0.0,
            pink: 0.2,
            brown: 0.6,
            rain: 0.2,
        };
        assert_eq!(mix.solo_style(), None);
        assert_eq!(mix.dominant(), SoundStyle::Brown);
        assert!((mix.total() - 1.0).abs() < 1e-6);

        // Ties resolve in SoundStyle::ALL order.
        let tie = SourceMix {
            white: 0.5,
            pink: 0.0,
            brown: 0.5,
            rain: 0.0,
        };
        assert_eq!(tie.dominant(), SoundStyle::White);

        let silent = SourceMix {
            white: 0.0,
            pink: 0.0,
            brown: 0.0,
            rain: 0.0,
        };
        assert_eq!(silent.dominant(), SoundStyle::White);
        assert_eq!(silent.solo_style(), None);
    }

    #[test]
    fn mix_describe_names_solos_and_lists_blends() {
        assert_eq!(SourceMix::solo(SoundStyle::Rain).describe(), "Rain");
        let blend = SourceMix {
            white: 0.0,
            pink: 0.0,
            brown: 0.4,
            rain: 0.6,
        };
        assert_eq!(blend.describe(), "Mix: Brown Noise 40% + Rain 60%");
    }

    #[test]
    fn non_finite_mix_levels_are_sanitized() {
        let mut settings = AudioSettings::default();
        settings.set_mix(SourceMix {
            white: f32::NAN,
            pink: 2.0,
            brown: -1.0,
            rain: 0.5,
        });
        let mix = settings.mix();
        assert_eq!(mix.white, 0.0);
        assert_eq!(mix.pink, 1.0);
        assert_eq!(mix.brown, 0.0);
        assert_eq!(mix.rain, 0.5);
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
