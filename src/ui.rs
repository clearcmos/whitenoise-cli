use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::settings::{AudioSettings, FREQUENCY_BANDS, slider_to_db};

const SLIDER_WIDTH: usize = 30;

pub struct InteractiveUi {
    settings: Arc<Mutex<AudioSettings>>,
    selected: usize,
    running: Arc<AtomicBool>,
}

impl InteractiveUi {
    pub fn new(settings: Arc<Mutex<AudioSettings>>, running: Arc<AtomicBool>) -> Self {
        Self {
            settings,
            selected: 0,
            running,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let _terminal = TerminalSession::enter()?;
        self.draw()?;

        while self.running.load(Ordering::Relaxed) {
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }

            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if self.handle_key(key) {
                        break;
                    }
                    self.draw()?;
                }
                Event::Resize(_, _) => self.draw()?,
                _ => {}
            }
        }
        Ok(())
    }

    fn draw(&self) -> Result<()> {
        let settings = *self
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stdout = io::stdout().lock();

        execute!(stdout, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        queue!(
            stdout,
            SetForegroundColor(Color::Cyan),
            Print("Whitenoise\r\n"),
            ResetColor,
            Print(format!(
                "Source: {} (S to switch)\r\n",
                settings.sound_style.label()
            )),
            Print(format!(
                "Listening contour: {} (N to toggle)\r\n",
                if settings.listening_contour {
                    "on"
                } else {
                    "off"
                }
            )),
            Print("Controls: Up/Down select, Left/Right adjust, R reset EQ, Q quit\r\n\r\n")
        )?;

        draw_slider(
            &mut stdout,
            "Volume",
            settings.volume,
            5,
            self.selected == 0,
            &format!("{:>3.0}%", settings.volume * 100.0),
        )?;

        for (index, band) in FREQUENCY_BANDS.iter().enumerate() {
            draw_slider(
                &mut stdout,
                band.name,
                settings.frequency_bands[index],
                6 + index as u16,
                self.selected == index + 1,
                &format!("{:+5.1} dB", slider_to_db(settings.frequency_bands[index])),
            )?;
        }

        queue!(
            stdout,
            cursor::MoveTo(2, 15),
            SetForegroundColor(Color::DarkGrey),
            Print("EQ range: -12 dB to +12 dB; center position is neutral."),
            cursor::MoveTo(2, 16),
            Print("Bands: ")
        )?;
        for (index, band) in FREQUENCY_BANDS.iter().enumerate() {
            if index == 4 {
                queue!(stdout, cursor::MoveTo(9, 17))?;
            }
            queue!(
                stdout,
                Print(format!(
                    "{} {:.0}-{:.0} Hz  ",
                    band.name, band.min_freq, band.max_freq
                ))
            )?;
        }
        queue!(stdout, ResetColor)?;
        stdout.flush()?;
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(FREQUENCY_BANDS.len());
            }
            KeyCode::Left => self.adjust_selected(-0.05),
            KeyCode::Right => self.adjust_selected(0.05),
            KeyCode::Char('n' | 'N') => {
                let mut settings = self.lock_settings();
                settings.listening_contour = !settings.listening_contour;
            }
            KeyCode::Char('s' | 'S') => {
                let mut settings = self.lock_settings();
                settings.sound_style = settings.sound_style.next();
            }
            KeyCode::Char('r' | 'R') => {
                self.lock_settings().frequency_bands = [0.5; FREQUENCY_BANDS.len()];
            }
            KeyCode::Char('q' | 'Q') | KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    fn adjust_selected(&self, amount: f32) {
        let mut settings = self.lock_settings();
        if self.selected == 0 {
            settings.volume = (settings.volume + amount).clamp(0.0, 1.0);
        } else {
            let band = &mut settings.frequency_bands[self.selected - 1];
            *band = (*band + amount).clamp(0.0, 1.0);
        }
    }

    fn lock_settings(&self) -> std::sync::MutexGuard<'_, AudioSettings> {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn draw_slider(
    stdout: &mut impl Write,
    name: &str,
    value: f32,
    row: u16,
    selected: bool,
    value_label: &str,
) -> Result<()> {
    let filled = (value.clamp(0.0, 1.0) * SLIDER_WIDTH as f32).round() as usize;
    queue!(stdout, cursor::MoveTo(2, row))?;

    if selected {
        queue!(
            stdout,
            SetForegroundColor(Color::Yellow),
            Print(format!("> {:<12}", name))
        )?;
    } else {
        queue!(
            stdout,
            SetForegroundColor(Color::White),
            Print(format!("  {:<12}", name))
        )?;
    }

    queue!(stdout, Print(" ["), SetForegroundColor(Color::Green))?;
    for _ in 0..filled {
        queue!(stdout, Print("#"))?;
    }
    queue!(stdout, SetForegroundColor(Color::DarkGrey))?;
    for _ in filled..SLIDER_WIDTH {
        queue!(stdout, Print("-"))?;
    }
    queue!(
        stdout,
        SetForegroundColor(Color::White),
        Print(format!("] {value_label}")),
        ResetColor
    )?;
    Ok(())
}

struct TerminalSession;

impl TerminalSession {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, cursor::Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
    }
}
