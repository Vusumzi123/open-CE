//! Skyrim Special Edition data-directory view.
//!
//! Scans the game `Data` directory, parses a `plugins.txt` load order when one
//! is available, and resolves assets through loose files and BSA archives.
//!
//! Archive priority is filename order for now. Real installations define BSA
//! order through `Skyrim.ini`; that mapping belongs to a later layer.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use oce_bsa::Archive;
use oce_core::{Error, Result};

pub const VANILLA_MASTERS: &[&str] = &[
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
];

pub const GAME_DIR_NAME: &str = "Skyrim Special Edition";
pub const STEAM_APP_ID: &str = "489830";

#[derive(Clone, Debug)]
pub struct DataDir {
    root: PathBuf,
    data: PathBuf,
}

impl DataDir {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = if is_game_data(path) {
            path.to_path_buf()
        } else {
            let candidate = path.join("Data");
            if is_game_data(&candidate) {
                candidate
            } else {
                return Err(Error::invalid(format!(
                    "{} is neither a Skyrim SE install nor a Data directory",
                    path.display()
                )));
            }
        };
        let root = if data.file_name().and_then(|value| value.to_str()) == Some("Data") {
            data.parent().unwrap_or(&data).to_path_buf()
        } else {
            data.clone()
        };
        Ok(Self { root, data })
    }

    /// Finds a Skyrim SE installation without an explicit path.
    ///
    /// `OCE_SKYRIM_SE` wins. Otherwise the current directory, its ancestors and
    /// the libraries listed in Steam's `libraryfolders.vdf` are checked.
    pub fn detect() -> Result<Self> {
        if let Some(value) = env::var_os("OCE_SKYRIM_SE") {
            return Self::open(PathBuf::from(value));
        }
        let candidates = detection_candidates();
        for candidate in &candidates {
            if let Ok(data) = Self::open(candidate) {
                return Ok(data);
            }
        }
        let mut message =
            String::from("Skyrim Special Edition not found; set OCE_SKYRIM_SE or pass a game path");
        if !candidates.is_empty() {
            message.push_str("\nsearched:");
            for path in candidates.iter().take(50) {
                message.push_str(&format!("\n  {}", path.display()));
            }
        }
        Err(Error::invalid(message))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self) -> &Path {
        &self.data
    }

    pub fn plugins(&self) -> Result<Vec<PathBuf>> {
        let mut plugins = self.files_with_extensions(&["esm", "esp", "esl"])?;
        plugins.sort_by_key(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
        });
        Ok(plugins)
    }

    pub fn archives(&self) -> Result<Vec<PathBuf>> {
        let mut archives = self.files_with_extensions(&["bsa"])?;
        archives.sort_by_key(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
        });
        Ok(archives)
    }

    pub fn find_plugin(&self, name: &str) -> Result<Option<PathBuf>> {
        if !name.is_empty() && name.contains(['/', '\\']) {
            let path = self.data.join(name.replace('\\', "/"));
            return Ok(path.is_file().then_some(path));
        }
        for path in self.plugins()? {
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    pub fn core_masters(&self) -> Vec<PathBuf> {
        VANILLA_MASTERS
            .iter()
            .filter_map(|name| self.find_plugin(name).ok().flatten())
            .collect()
    }

    /// Resolves a loose asset path. Game paths use backslashes; the host path
    /// uses the platform separator.
    pub fn loose_path(&self, asset: &str) -> Option<PathBuf> {
        let relative = asset.replace('\\', "/");
        if relative.starts_with('/') || relative.split('/').any(|part| part == "..") {
            return None;
        }
        let path = self.data.join(relative);
        path.is_file().then_some(path)
    }

    pub fn plugins_txt(&self) -> Option<PathBuf> {
        let mut candidates = vec![self.root.join("plugins.txt"), self.data.join("plugins.txt")];
        if let Some(steamapps) = self.steamapps_dir() {
            push_prefix_plugins(
                &mut candidates,
                &steamapps.join("compatdata").join(STEAM_APP_ID),
            );
        }
        if let Some(prefix) = env::var_os("STEAM_COMPAT_DATA_PATH") {
            push_prefix_plugins(&mut candidates, Path::new(&prefix));
        }
        candidates.into_iter().find(|path| path.is_file())
    }

    fn steamapps_dir(&self) -> Option<PathBuf> {
        let common = self.root.parent()?;
        let steamapps = common.parent()?;
        steamapps
            .file_name()
            .is_some_and(|name| name == "steamapps")
            .then(|| steamapps.to_path_buf())
    }

    fn files_with_extensions(&self, extensions: &[&str]) -> Result<Vec<PathBuf>> {
        let entries = fs::read_dir(&self.data).map_err(|error| {
            Error::invalid(format!(
                "cannot read data directory {}: {error}",
                self.data.display()
            ))
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if extensions.iter().any(|candidate| *candidate == extension) {
                files.push(path);
            }
        }
        Ok(files)
    }
}

#[derive(Clone, Debug)]
pub struct LoadOrderEntry {
    pub name: String,
    pub enabled: bool,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct LoadOrder {
    pub entries: Vec<LoadOrderEntry>,
}

impl LoadOrder {
    /// Known base-game order, restricted to plugins that actually exist.
    pub fn vanilla(data: &DataDir) -> Self {
        let entries = VANILLA_MASTERS
            .iter()
            .filter_map(|name| {
                data.find_plugin(name)
                    .ok()
                    .flatten()
                    .map(|path| LoadOrderEntry {
                        name: (*name).to_string(),
                        enabled: true,
                        path: Some(path),
                    })
            })
            .collect();
        Self { entries }
    }

    pub fn from_plugins_txt(data: &DataDir, path: impl AsRef<Path>) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref()).map_err(|error| {
            Error::invalid(format!(
                "cannot read load order {}: {error}",
                path.as_ref().display()
            ))
        })?;
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (enabled, name) = if let Some(rest) = line.strip_prefix('*') {
                (true, rest.trim())
            } else {
                (false, line)
            };
            if name.is_empty() {
                continue;
            }
            entries.push(LoadOrderEntry {
                name: name.to_string(),
                enabled,
                path: data.find_plugin(name)?,
            });
        }
        Ok(Self { entries })
    }

    pub fn enabled(&self) -> impl Iterator<Item = &LoadOrderEntry> {
        self.entries.iter().filter(|entry| entry.enabled)
    }
}

/// A loaded game directory with its load order and asset resolver.
pub struct Game {
    pub data: DataDir,
    pub load_order: LoadOrder,
    pub assets: AssetResolver,
}

impl Game {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::from_data(DataDir::open(root)?)
    }

    pub fn detect() -> Result<Self> {
        Self::from_data(DataDir::detect()?)
    }

    fn from_data(data: DataDir) -> Result<Self> {
        let load_order = match data.plugins_txt() {
            Some(path) => LoadOrder::from_plugins_txt(&data, path)?,
            None => LoadOrder::vanilla(&data),
        };
        let assets = AssetResolver::new(data.clone());
        Ok(Self {
            data,
            load_order,
            assets,
        })
    }

    pub fn load_archives(&mut self) -> Result<ArchiveLoadReport> {
        self.assets.load_all_archives()
    }

    pub fn plugin(&self, name: &str) -> Result<Option<oce_plugin::Plugin>> {
        match self.data.find_plugin(name)? {
            Some(path) => oce_plugin::Plugin::open(path).map(Some),
            None => Ok(None),
        }
    }

    pub fn plugin_header(&self, name: &str) -> Result<Option<oce_plugin::PluginHeader>> {
        match self.data.find_plugin(name)? {
            Some(path) => oce_plugin::read_header(path).map(Some),
            None => Ok(None),
        }
    }

    pub fn read_asset(&self, asset: &str) -> Result<Option<Vec<u8>>> {
        self.assets.read(asset)
    }
}

#[derive(Clone, Debug)]
pub enum AssetLocation {
    Loose(PathBuf),
    Archive { archive: PathBuf, path: String },
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveLoadReport {
    pub loaded: usize,
    pub failed: Vec<(PathBuf, String)>,
}

pub struct AssetResolver {
    data: DataDir,
    archives: Vec<Archive>,
}

impl AssetResolver {
    pub fn new(data: DataDir) -> Self {
        Self {
            data,
            archives: Vec::new(),
        }
    }

    pub fn data(&self) -> &DataDir {
        &self.data
    }

    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    pub fn add_archive(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.archives.push(Archive::open(path)?);
        Ok(())
    }

    /// Loads every `.bsa` in the data directory. Later archives take priority,
    /// matching the usual "later archive wins" load-order convention.
    pub fn load_all_archives(&mut self) -> Result<ArchiveLoadReport> {
        let mut report = ArchiveLoadReport::default();
        for path in self.data.archives()? {
            match Archive::open(&path) {
                Ok(archive) => {
                    report.loaded += 1;
                    self.archives.push(archive);
                }
                Err(error) => report.failed.push((path, error.to_string())),
            }
        }
        Ok(report)
    }

    pub fn find(&self, asset: &str) -> Option<AssetLocation> {
        if let Some(path) = self.data.loose_path(asset) {
            return Some(AssetLocation::Loose(path));
        }
        for archive in self.archives.iter().rev() {
            if let Some(index) = archive.find(asset) {
                return Some(AssetLocation::Archive {
                    archive: archive.path().to_path_buf(),
                    path: archive.files()[index].path.clone(),
                });
            }
        }
        None
    }

    pub fn read(&self, asset: &str) -> Result<Option<Vec<u8>>> {
        if let Some(path) = self.data.loose_path(asset) {
            return fs::read(path).map(Some).map_err(Into::into);
        }
        for archive in self.archives.iter().rev() {
            if let Some(index) = archive.find(asset) {
                return archive.read_file(index).map(Some);
            }
        }
        Ok(None)
    }
}

fn push_prefix_plugins(candidates: &mut Vec<PathBuf>, prefix: &Path) {
    let users = prefix.join("pfx/drive_c/users");
    let Ok(entries) = fs::read_dir(users) else {
        return;
    };
    for entry in entries.flatten() {
        let user = entry.path();
        candidates.push(user.join("AppData/Local/Skyrim Special Edition/plugins.txt"));
        candidates.push(user.join("AppData/Local/Skyrim Special Edition/Plugins.txt"));
    }
}

fn is_game_data(path: &Path) -> bool {
    path.is_dir() && (path.join("Skyrim.esm").is_file() || path.join("Skyrim - Misc.bsa").is_file())
}

fn detection_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        for ancestor in cwd.ancestors() {
            candidates.push(ancestor.to_path_buf());
            candidates.push(ancestor.join(GAME_DIR_NAME));
        }
    }
    if let Some(home) = home_dir() {
        for root in steam_roots(&home) {
            push_steam_game(&mut candidates, &root);
        }
    }
    for file in library_folders_files() {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for library in parse_library_folders(&text) {
            push_steam_game(&mut candidates, &library);
        }
    }
    let mut seen = HashSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

fn push_steam_game(candidates: &mut Vec<PathBuf>, library: &Path) {
    candidates.push(library.join("steamapps/common").join(GAME_DIR_NAME));
    candidates.push(library.join("SteamApps/common").join(GAME_DIR_NAME));
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn steam_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".local/share/Steam"),
        home.join("Library/Application Support/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    if cfg!(windows) {
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots.push(PathBuf::from(r"C:\Program Files\Steam"));
    }
    roots
}

fn library_folders_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = home_dir() {
        for root in steam_roots(&home) {
            files.push(root.join("steamapps/libraryfolders.vdf"));
            files.push(root.join("SteamApps/libraryfolders.vdf"));
        }
    }
    files
}

fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"path\"")?;
            let value = quoted_value(rest)?;
            Some(PathBuf::from(unescape_vdf(value)))
        })
        .collect()
}

fn quoted_value(input: &str) -> Option<&str> {
    let start = input.find('"')? + 1;
    let rest = &input[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn unescape_vdf(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            if let Some(escaped) = chars.next() {
                result.push(escaped);
            }
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_data() -> Option<PathBuf> {
        let root = std::env::var_os("OCE_SKYRIM_SE").map(PathBuf::from)?;
        let data = if root.join("Data").is_dir() {
            root.join("Data")
        } else {
            root
        };
        data.join("Skyrim.esm").exists().then_some(data)
    }

    #[test]
    fn real_data_directory_if_present() {
        let Some(data) = game_data() else {
            eprintln!("skipping real data-directory test: game data not found");
            return;
        };
        let data = DataDir::open(&data).unwrap();
        let plugins = data.plugins().unwrap();
        assert!(plugins.len() >= 5);
        for path in &plugins {
            oce_plugin::read_header(path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
        assert!(!data.archives().unwrap().is_empty());
        assert!(data.find_plugin("skyrim.esm").unwrap().is_some());

        let mut resolver = AssetResolver::new(data);
        let report = resolver.load_all_archives().unwrap();
        assert!(report.loaded > 0);
        for (path, error) in &report.failed {
            eprintln!("archive failed: {}: {error}", path.display());
        }
        resolver
            .add_archive(resolver.data().path().join("Skyrim - Textures0.bsa"))
            .unwrap();
        let location = resolver.find("textures\\actors\\dog\\dog.dds").unwrap();
        assert!(matches!(location, AssetLocation::Archive { .. }));
        let bytes = resolver
            .read("textures\\actors\\dog\\dog.dds")
            .unwrap()
            .unwrap();
        assert!(bytes.starts_with(b"DDS "));
    }

    #[test]
    fn parses_steam_library_folders() {
        let text = r#""libraryfolders"
{
    "0"
    {
        "path" "/home/user/.local/share/Steam"
    }
    "1"
    {
        "path" "D:\\Games\\Steam"
    }
}"#;
        assert_eq!(
            parse_library_folders(text),
            vec![
                PathBuf::from("/home/user/.local/share/Steam"),
                PathBuf::from(r"D:\Games\Steam"),
            ]
        );
    }

    fn temporary_root(label: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("oce-{label}-{}-{stamp}", std::process::id()))
    }

    #[test]
    fn accepts_game_root_and_data_directory() {
        let root = temporary_root("accept");
        let data = root.join("Data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("Skyrim.esm"), b"").unwrap();

        let from_root = DataDir::open(&root).unwrap();
        assert_eq!(from_root.root(), root.as_path());
        assert_eq!(from_root.path(), data.as_path());

        let from_data = DataDir::open(&data).unwrap();
        assert_eq!(from_data.path(), data.as_path());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finds_plugins_txt_in_proton_prefix() {
        let root = temporary_root("prefix");
        let game = root.join("steamapps/common").join(GAME_DIR_NAME);
        let data = game.join("Data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("Skyrim.esm"), b"").unwrap();
        let prefix = root
            .join("steamapps/compatdata")
            .join(STEAM_APP_ID)
            .join("pfx/drive_c/users/steamuser/AppData/Local")
            .join(GAME_DIR_NAME);
        fs::create_dir_all(&prefix).unwrap();
        fs::write(prefix.join("plugins.txt"), b"*Skyrim.esm\n").unwrap();

        let data_dir = DataDir::open(&game).unwrap();
        assert_eq!(data_dir.plugins_txt(), Some(prefix.join("plugins.txt")));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejects_directory_without_game_data() {
        let root = temporary_root("reject");
        fs::create_dir_all(root.join("Data")).unwrap();
        assert!(DataDir::open(&root).is_err());
        fs::remove_dir_all(&root).ok();
    }
}
