//! USB live lighting with a durable checkpoint and exact restoration.

use crate::music::Shared;
use aula_studio::{
    device::{DeviceStatus, Keyboard},
    protocol::{self, Lighting, Transport},
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(serde::Serialize, serde::Deserialize)]
struct Recovery {
    version: u8,
    settings: Vec<u8>,
    colors: Vec<[u8; 3]>,
    keymaps: Vec<Vec<u8>>,
}

impl Recovery {
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.colors.len() != 126 || self.keymaps.len() != 2 {
            return Err("Invalid music recovery checkpoint shape".into());
        }
        Lighting::read(&self.settings).map_err(|error| error.to_string())?;
        for map in &self.keymaps {
            protocol::normalize_keymap(map, Transport::Usb).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn colors(&self) -> Result<[[u8; 3]; 126], String> {
        self.colors
            .as_slice()
            .try_into()
            .map_err(|_| "Invalid music recovery colors".into())
    }

    fn save(&self) -> Result<PathBuf, String> {
        self.validate()?;
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("Cannot save recovery checkpoint: LOCALAPPDATA is unavailable")?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("Cannot timestamp recovery checkpoint: {error}"))?
            .as_nanos();
        let directory = local.join("AulaStudio").join("recovery");
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Cannot create recovery directory: {error}"))?;
        let path = directory.join(format!("music-{timestamp}.json"));
        let data = serde_json::to_vec(self)
            .map_err(|error| format!("Cannot serialize music recovery checkpoint: {error}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "Cannot create recovery checkpoint {}: {error}",
                    path.display()
                )
            })?;
        file.write_all(&data)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!(
                    "Cannot durably save recovery checkpoint {}: {error}",
                    path.display()
                )
            })?;
        Ok(path)
    }
}

pub struct LiveRgb {
    shared: Shared,
    original: Recovery,
    checkpoint: PathBuf,
    last_sent: Option<Instant>,
    last_colors: Option<[[u8; 3]; 126]>,
    faulted: bool,
}

impl LiveRgb {
    pub fn start(board: &mut Keyboard, shared: Shared) -> Result<Self, String> {
        if board.transport != Transport::Usb {
            return Err("Music sync requires the wired USB connection".into());
        }
        {
            let state = shared
                .lock()
                .map_err(|_| "Audio worker state is unavailable; lighting was not changed")?;
            if !state.active || !state.ready {
                return Err("Audio capture is not ready; lighting was not changed".into());
            }
        }
        let original = Recovery {
            version: 1,
            settings: board.settings()?,
            colors: board.read_colors()?.to_vec(),
            keymaps: vec![board.keymap(0)?, board.keymap(1)?],
        };
        original.validate()?;
        let previous = Lighting::read(&original.settings).map_err(|error| error.to_string())?;
        let temporary = Lighting {
            effect: 21,
            brightness: 9,
            ..previous
        }
        .apply(&original.settings)
        .map_err(|error| error.to_string())?;
        let checkpoint = original.save()?;
        let live = Self {
            shared,
            original,
            checkpoint,
            last_sent: None,
            last_colors: None,
            faulted: false,
        };

        // Capture can stop while the read-only snapshot is being saved. Check
        // again immediately before the first setting write.
        {
            let state = live.shared.lock().map_err(|_| {
                format!(
                    "Audio worker state is unavailable; lighting was not changed. Checkpoint: {}",
                    live.checkpoint.display()
                )
            })?;
            if !state.active || !state.ready {
                return Err(format!(
                    "Audio capture stopped; lighting was not changed. Checkpoint: {}",
                    live.checkpoint.display()
                ));
            }
        }
        let entered = board.write(4, 0, &temporary).and_then(|_| {
            if board.settings()? != temporary {
                return Err("Temporary music lighting settings did not match readback".into());
            }
            Ok(())
        });
        if let Err(error) = entered {
            let path = live.checkpoint.clone();
            return Err(match live.restore(board) {
                Ok(_) => format!(
                    "Music sync could not start: {error}. Original lighting was restored and verified. Checkpoint: {}",
                    path.display()
                ),
                Err(recovery) => {
                    format!("Music sync could not start: {error}. Recovery failed: {recovery}")
                }
            });
        }
        Ok(live)
    }

    /// Latest frame only. A failed transfer permanently disables further frame
    /// writes in this session; the worker must call `stop` to restore settings.
    pub fn tick(&mut self, board: &Keyboard) -> Result<bool, String> {
        if self.faulted {
            return Err(format!(
                "Music output stopped after an uncertain transfer; restore before restarting. Checkpoint: {}",
                self.checkpoint.display()
            ));
        }
        let (active, ready, frames, colors) = {
            let state = self
                .shared
                .lock()
                .map_err(|_| "Audio worker state is unavailable")?;
            (state.active, state.ready, state.frames, state.colors)
        };
        if !active {
            return Ok(false);
        }
        if !ready || frames == 0 {
            return Ok(true);
        }
        let now = Instant::now();
        if !frame_due(self.last_sent, now, self.last_colors == Some(colors)) {
            return Ok(true);
        }
        if let Err(error) = board.stream_colors(&colors) {
            self.faulted = true;
            return Err(format!(
                "{error}. No further live frames will be sent. Checkpoint: {}",
                self.checkpoint.display()
            ));
        }
        self.last_sent = Some(Instant::now());
        self.last_colors = Some(colors);
        match self.shared.lock() {
            Ok(mut state) => state.usb_frames = state.usb_frames.saturating_add(1),
            Err(_) => {
                self.faulted = true;
                return Err(format!(
                    "A live frame was accepted but audio worker state is unavailable; output stopped. Checkpoint: {}",
                    self.checkpoint.display()
                ));
            }
        }
        Ok(true)
    }

    pub fn stop(self, board: &mut Keyboard) -> Result<DeviceStatus, String> {
        self.restore(board)
    }

    pub fn checkpoint(&self) -> &Path {
        &self.checkpoint
    }

    fn restore(&self, board: &mut Keyboard) -> Result<DeviceStatus, String> {
        let result = self.restore_checked(board);
        result.map_err(|error| {
            format!(
                "{error}. Recovery checkpoint retained at {}. Recover over USB with f75-check --recover \"{}\"",
                self.checkpoint.display(),
                self.checkpoint.display()
            )
        })
    }

    fn restore_checked(&self, board: &mut Keyboard) -> Result<DeviceStatus, String> {
        self.original.validate()?;
        if board.transport != Transport::Usb {
            return Err("Music recovery requires USB".into());
        }
        let colors = self.original.colors()?;
        let mut errors = Vec::new();
        match board.read_colors() {
            Ok(current) if current != colors => {
                let data = protocol::color_transfer(&colors, Transport::Usb)
                    .map_err(|error| error.to_string())?;
                if let Err(error) = board.write(6, 0, &data) {
                    errors.push(format!("Original RGB restoration failed: {error}"));
                }
            }
            Ok(_) => {}
            Err(error) => errors.push(format!("Could not compare saved RGB: {error}")),
        }
        // Always attempt the original raw block, even if color restoration
        // failed, so a partial error does not prevent leaving live mode.
        if let Err(error) = board.write(4, 0, &self.original.settings) {
            errors.push(format!("Original settings restoration failed: {error}"));
        }
        match board.settings() {
            Ok(settings) if settings == self.original.settings => {}
            Ok(_) => errors.push("Restored raw settings did not match the checkpoint".into()),
            Err(error) => errors.push(format!("Restored settings readback failed: {error}")),
        }
        match board.read_colors() {
            Ok(current) if current == colors => {}
            Ok(_) => errors.push("Restored RGB did not match the checkpoint".into()),
            Err(error) => errors.push(format!("Restored RGB readback failed: {error}")),
        }
        for (layer, original) in self.original.keymaps.iter().enumerate() {
            match board.keymap(layer as u8) {
                Ok(map) if map == *original => {}
                Ok(_) => errors.push(format!("Keymap layer {layer} changed during music sync")),
                Err(error) => errors.push(format!("Keymap layer {layer} readback failed: {error}")),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        Ok(DeviceStatus {
            transport: board.transport,
            product: board.product.clone(),
            lighting: Lighting::read(&self.original.settings).map_err(|error| error.to_string())?,
            sleep_seconds: protocol::SLEEP
                .iter()
                .find(|(_, value)| *value == self.original.settings[24])
                .map(|(seconds, _)| *seconds),
        })
    }
}

fn frame_due(previous: Option<Instant>, now: Instant, unchanged: bool) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    let elapsed = now.saturating_duration_since(previous);
    elapsed >= FRAME_INTERVAL && (!unchanged || elapsed >= KEEPALIVE_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Recovery {
        let mut settings = vec![0; 128];
        settings[126..].copy_from_slice(&[0x5a, 0xa5]);
        let settings = Lighting {
            effect: 3,
            brightness: 7,
            speed: 2,
            color: 1,
        }
        .apply(&settings)
        .unwrap();
        let mut map = vec![0; 512];
        map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        Recovery {
            version: 1,
            settings,
            colors: vec![[13, 29, 47]; 126],
            keymaps: vec![map.clone(), map],
        }
    }

    #[test]
    fn checkpoint_round_trip_preserves_raw_settings_rgb_and_both_keymaps() {
        let mut source = snapshot();
        source.settings[110] = 97;
        source.keymaps[0][400..404].copy_from_slice(&[0, 2, 0, 20]);
        source.keymaps[1][120..124].copy_from_slice(&[3, 0, 0, 5]);
        source.colors[125] = [255, 127, 19];
        source.validate().unwrap();
        let bytes = serde_json::to_vec(&source).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(value.get("macro_bank").is_none());
        let restored: Recovery = serde_json::from_slice(&bytes).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.settings, source.settings);
        assert_eq!(restored.colors, source.colors);
        assert_eq!(restored.keymaps, source.keymaps);
        assert_eq!(restored.colors().unwrap()[125], [255, 127, 19]);
    }

    #[test]
    fn incomplete_and_invalid_checkpoints_are_rejected() {
        let mut source = snapshot();
        source.version = 2;
        assert!(source.validate().is_err());
        source = snapshot();
        source.colors.pop();
        assert!(source.validate().is_err());
        assert!(source.colors().is_err());
        source = snapshot();
        source.settings[127] = 0;
        assert!(source.validate().is_err());
        source = snapshot();
        source.keymaps[1].pop();
        assert!(source.validate().is_err());
        source = snapshot();
        source.keymaps.pop();
        assert!(source.validate().is_err());
    }

    #[test]
    fn frame_pacing_caps_twenty_fps_and_unchanged_keepalive() {
        let start = Instant::now();
        assert!(frame_due(None, start, false));
        assert!(!frame_due(
            Some(start),
            start + Duration::from_millis(49),
            false
        ));
        assert!(frame_due(
            Some(start),
            start + Duration::from_millis(50),
            false
        ));
        assert!(!frame_due(
            Some(start),
            start + Duration::from_millis(499),
            true
        ));
        assert!(frame_due(
            Some(start),
            start + Duration::from_millis(500),
            true
        ));
        assert!(!frame_due(
            Some(start),
            start - Duration::from_millis(1),
            false
        ));
    }
}
