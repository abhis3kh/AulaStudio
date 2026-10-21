use aula_studio::protocol::Lighting;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_DOCUMENT_BYTES: usize = 2_000_000;

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub lighting: [u8; 4],
    pub sleep_seconds: u16,
    pub colors: Vec<[u8; 3]>,
    #[serde(default)]
    pub keymaps: Vec<Vec<u8>>,
    #[serde(default)]
    pub macros: Vec<aula_studio::macros::MacroBinding>,
}
impl Profile {
    pub fn new(
        name: String,
        lighting: Lighting,
        sleep_seconds: u16,
        colors: &[[u8; 3]; 126],
    ) -> Self {
        Self {
            name,
            lighting: [
                lighting.effect,
                lighting.brightness,
                lighting.speed,
                lighting.color,
            ],
            sleep_seconds,
            colors: colors.to_vec(),
            keymaps: Vec::new(),
            macros: Vec::new(),
        }
    }
    pub fn lighting(&self) -> Lighting {
        Lighting {
            effect: self.lighting[0],
            brightness: self.lighting[1],
            speed: self.lighting[2],
            color: self.lighting[3],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 80 {
            return Err("Profile names must contain 1 to 80 characters".into());
        }
        self.lighting().validate().map_err(|e| e.to_string())?;
        if !aula_studio::protocol::SLEEP
            .iter()
            .any(|(seconds, _)| *seconds == self.sleep_seconds)
        {
            return Err("Unsupported profile sleep timeout".into());
        }
        if self.colors.len() != 126 {
            return Err("Profile must contain 126 matrix colors".into());
        }
        if self.keymaps.len() > 2
            || self
                .keymaps
                .iter()
                .any(|map| map.len() != 512 || map[508..] != [0, 0, 0x5a, 0xa5])
        {
            return Err("Invalid profile keymap".into());
        }
        if !self.macros.is_empty() {
            aula_studio::macros::encode(&self.macros)?;
        }
        Ok(())
    }

    /// Older presets remain readable even when their hardware macro bank was
    /// not captured. Applying one must never reuse an unrelated current bank.
    pub fn validate_for_apply(&self) -> Result<(), String> {
        self.validate()?;
        for (layer, map) in self.keymaps.iter().enumerate() {
            for (matrix, code) in map[..504].as_chunks::<4>().0.iter().enumerate() {
                if code[0] != 3 {
                    continue;
                }
                if layer != 0 {
                    return Err("This preset contains Fn-layer macro references that cannot be restored from its base-layer macro definitions. Remove those references before applying.".into());
                }
                let binding = self.macros.get(code[3] as usize).ok_or(
                    "This preset references hardware macros whose definitions were not saved. Recreate those macros before applying.",
                )?;
                if code[1..3] != [1, 1] || binding.matrix != matrix {
                    return Err("This preset's macro keymap references do not match its saved macro definitions. Correct the preset before applying.".into());
                }
            }
        }
        Ok(())
    }

    /// Macro assignments are installed only after their bank has been uploaded
    /// and verified; profile keymap writes must not activate stale references.
    pub fn keymaps_for_apply(&self) -> Result<Vec<Vec<u8>>, String> {
        self.validate_for_apply()?;
        self.keymaps
            .iter()
            .map(|map| {
                aula_studio::protocol::without_macros(map).map_err(|error| error.to_string())
            })
            .collect()
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Document {
    version: u8,
    profiles: Vec<Profile>,
}

pub fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("AulaStudio")
        .join("profiles.json")
}
pub fn load(path: &Path) -> Result<Vec<Profile>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Cannot read profiles: {e}")),
    };
    let mut data = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| format!("Cannot read profiles: {error}"))?;
    if data.len() > MAX_DOCUMENT_BYTES {
        return Err("Profile file exceeds the 2 MB limit".into());
    }
    let document: Document =
        serde_json::from_slice(&data).map_err(|e| format!("Cannot parse profiles: {e}"))?;
    if document.version != 1 {
        return Err("Unsupported profile file version".into());
    }
    for profile in &document.profiles {
        profile.validate()?;
    }
    Ok(document.profiles)
}
pub fn save(path: &Path, profiles: &[Profile]) -> Result<(), String> {
    for profile in profiles {
        profile.validate()?;
    }
    let document = Document {
        version: 1,
        profiles: profiles.to_vec(),
    };
    let data = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    if data.len() > MAX_DOCUMENT_BYTES {
        return Err("Cannot save profiles: profile file exceeds the 2 MB limit. Remove a preset before saving.".into());
    }
    let parent = path.parent().ok_or("Profile path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create profile folder: {e}"))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&data).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Cannot save profiles: {e}"))
}
#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Replace the existing document atomically, retaining the old file if replacement fails.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 1 | 8) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Profile {
        Profile::new(
            "Desk".into(),
            Lighting {
                effect: 1,
                brightness: 9,
                speed: 2,
                color: 0,
            },
            300,
            &[[10, 20, 30]; 126],
        )
    }

    fn keymap() -> Vec<u8> {
        let mut map = vec![0; 512];
        map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        map
    }

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("aula-profile-{name}-{}", std::process::id()))
            .join("profiles.json")
    }

    #[test]
    fn profiles_roundtrip_and_replace() {
        let path = std::env::temp_dir()
            .join(format!("aula-profile-test-{}", std::process::id()))
            .join("profiles.json");
        let mut profile = Profile::new(
            "Desk".into(),
            Lighting {
                effect: 1,
                brightness: 9,
                speed: 2,
                color: 0,
            },
            300,
            &[[10, 20, 30]; 126],
        );
        save(&path, &[profile.clone()]).unwrap();
        profile.name = "Evening".into();
        save(&path, &[profile.clone()]).unwrap();
        assert_eq!(load(&path).unwrap()[0].name, "Evening");
        let previous = fs::read(&path).unwrap();
        profile.name.clear();
        assert!(save(&path, &[profile]).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn invalid_profile_rejected_before_write() {
        let profile = Profile::new(
            "".into(),
            Lighting {
                effect: 19,
                brightness: 9,
                speed: 2,
                color: 0,
            },
            300,
            &[[0; 3]; 126],
        );
        assert!(profile.validate().is_err());
    }

    #[test]
    fn document_size_boundary_accepts_limit_and_rejects_one_extra_byte() {
        let path = test_path("size-boundary");
        save(&path, &[fixture()]).unwrap();
        let mut data = fs::read(&path).unwrap();
        data.resize(MAX_DOCUMENT_BYTES, b' ');
        fs::write(&path, &data).unwrap();
        assert_eq!(load(&path).unwrap()[0].name, "Desk");
        data.push(b' ');
        fs::write(&path, data).unwrap();
        assert!(load(&path).err().unwrap().contains("2 MB limit"));
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn oversized_save_preserves_the_existing_readable_document() {
        let path = test_path("oversized-save");
        let mut profile = fixture();
        profile.keymaps = vec![keymap(), keymap()];
        save(&path, &[profile.clone()]).unwrap();
        let original = fs::read(&path).unwrap();
        let oversized = vec![profile; 100];
        assert!(save(&path, &oversized).unwrap_err().contains("2 MB limit"));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(load(&path).unwrap().len(), 1);
        assert!(
            !path
                .with_extension(format!("{}.tmp", std::process::id()))
                .exists()
        );
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn omitted_optional_fields_remain_compatible_with_older_profiles() {
        let mut document = serde_json::to_value(fixture()).unwrap();
        document.as_object_mut().unwrap().remove("keymaps");
        document.as_object_mut().unwrap().remove("macros");
        let profile: Profile = serde_json::from_value(document).unwrap();
        assert!(profile.keymaps.is_empty() && profile.macros.is_empty());
        assert!(profile.validate_for_apply().is_ok());
    }

    #[test]
    fn uncaptured_macro_references_remain_loadable_but_cannot_be_applied() {
        let mut profile = fixture();
        let mut map = keymap();
        map[78 * 4..78 * 4 + 4].copy_from_slice(&[3, 1, 1, 0]);
        profile.keymaps = vec![map];
        assert!(profile.validate().is_ok());
        assert!(
            profile
                .validate_for_apply()
                .unwrap_err()
                .contains("not saved")
        );
        assert!(profile.keymaps_for_apply().is_err());
    }

    #[test]
    fn macro_references_must_match_the_saved_base_layer_definitions() {
        let mut profile = fixture();
        profile.macros.push(aula_studio::macros::MacroBinding {
            name: "Text".into(),
            matrix: 78,
            events: aula_studio::macros::text_events("a", 20).unwrap(),
        });
        let mut base = keymap();
        base[78 * 4..78 * 4 + 4].copy_from_slice(&[3, 1, 1, 0]);
        base[4..8].copy_from_slice(&[0, 0, 0, 4]);
        profile.keymaps = vec![base.clone(), keymap()];
        let prepared = profile.keymaps_for_apply().unwrap();
        assert_eq!(&prepared[0][78 * 4..78 * 4 + 4], &[0; 4]);
        assert_eq!(&prepared[0][4..8], &[0, 0, 0, 4]);
        assert_eq!(profile.keymaps[0], base);

        profile.keymaps[0][78 * 4 + 3] = 1;
        assert!(profile.validate_for_apply().is_err());
        profile.keymaps[0][78 * 4 + 3] = 0;
        profile.macros[0].matrix = 77;
        assert!(profile.validate_for_apply().is_err());
        profile.macros[0].matrix = 78;
        profile.keymaps[0][78 * 4 + 1] = 0;
        assert!(profile.validate_for_apply().is_err());
        profile.keymaps[0][78 * 4 + 1] = 1;
        profile.keymaps[1][78 * 4..78 * 4 + 4].copy_from_slice(&[3, 1, 1, 0]);
        assert!(
            profile
                .validate_for_apply()
                .unwrap_err()
                .contains("Fn-layer")
        );
    }

    #[test]
    fn new_authored_macros_need_not_be_present_in_a_draft_keymap() {
        let mut profile = fixture();
        profile.macros.push(aula_studio::macros::MacroBinding {
            name: "New".into(),
            matrix: 78,
            events: aula_studio::macros::text_events("a", 20).unwrap(),
        });
        assert!(profile.validate_for_apply().is_ok());
        profile.keymaps = vec![keymap()];
        assert!(profile.keymaps_for_apply().is_ok());
    }
}
