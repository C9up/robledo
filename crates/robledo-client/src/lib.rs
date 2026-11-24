//! robledo's client, on the phone.
//!
//! An update is a manifest — its id, the native runtime it needs, the file to
//! launch, and every file with its SHA-256 and size — signed with Ed25519 by
//! `ream android:publish`, and the files it lists. This crate:
//!
//! - **stages** one: checks the signature against the public key built into
//!   the app — before any file is fetched — the runtime against the app's
//!   own, that it is newer than every update this phone accepted and than the
//!   bundle inside the app (a replayed old update is refused), each file
//!   against its digest and size as it arrives, and writes it under
//!   `bundles/<id>` beside a `pending` mark — nothing a network or a server
//!   could alter or replay is ever launched;
//! - **launches** at the next start: a pending update becomes current, on
//!   trial; the previous one is kept;
//! - **rolls back**: an update on trial that never reported ready (it crashed
//!   before its first screen) is abandoned at the start after, recorded as
//!   failed so it is not staged again, and the previous one launches;
//! - is told **ready** by the app once it showed its first screen, which ends
//!   the trial and prunes the bundles no longer needed.
//!
//! The state lives in `state.json`, replaced atomically. The platform layer
//! (Kotlin, Swift later) only reads the app's config, hands over bytes and
//! launches the path this returns.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub mod ffi;

/// The largest manifest a phone reads (its bytes), and the most files it may
/// list: a manifest is checked before anything it names is fetched, and these
/// bound what checking it costs.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_FILES: usize = 4096;

/// What the app was built with (`ream android:*` writes it into the app).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub url: String,
    pub public_key: [u8; 32],
    pub runtime: String,
    pub platform: String,
    /// When the bundle inside the app was built (`embeddedCreatedAt`, seconds):
    /// an update not newer than it is refused, and dropped at launch once a
    /// store release brings a newer bundle. Absent from builds that do not
    /// write it.
    pub embedded_created_at: Option<u64>,
    /// The app's id (`appId`): when the build writes it, an update must name
    /// the same app. Absent from builds that do not write it.
    pub app_id: Option<String>,
}

pub fn parse_config(bytes: &[u8]) -> Result<Config, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("config: {e}"))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("config: `{key}` is missing"))
    };
    let key = base64::engine::general_purpose::STANDARD
        .decode(text("publicKey")?)
        .map_err(|_| "config: `publicKey` is not base64".to_string())?;
    let public_key: [u8; 32] = key
        .try_into()
        .map_err(|_| "config: `publicKey` is not a 32-byte Ed25519 key".to_string())?;
    VerifyingKey::from_bytes(&public_key)
        .map_err(|_| "config: `publicKey` is not a valid Ed25519 key".to_string())?;
    let embedded_created_at = match value.get("embeddedCreatedAt") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or("config: `embeddedCreatedAt` is not a number of seconds")?,
        ),
    };
    let app_id = match value.get("appId") {
        None | Some(Value::Null) => None,
        Some(_) => Some(text("appId")?),
    };
    Ok(Config {
        url: text("url")?,
        public_key,
        runtime: text("runtimeVersion")?,
        platform: text("platform")?,
        embedded_created_at,
        app_id,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub path: String,
    pub sha256: [u8; 32],
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    /// When `ream android:publish` built it (seconds): what orders updates.
    pub created_at: u64,
    /// The app it is for, when the publisher names one.
    pub app_id: Option<String>,
    pub runtime: String,
    pub platform: String,
    pub launch: String,
    pub files: Vec<FileEntry>,
}

/// An id names a directory: letters, digits, `-` and `_` only.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A file path stays inside its bundle: relative, forward slashes, no `..`.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.contains('\\')
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(format!("manifest: larger than {MAX_MANIFEST_BYTES} bytes"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("manifest: {e}"))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("manifest: `{key}` is missing"))
    };
    let id = text("id")?;
    if !valid_id(&id) {
        return Err(format!("manifest: id {id:?} is not a bundle id"));
    }
    let created_at = value
        .get("createdAt")
        .and_then(Value::as_u64)
        .filter(|&t| t > 0)
        .ok_or("manifest: `createdAt` is missing")?;
    let app_id = match value.get("appId") {
        None | Some(Value::Null) => None,
        Some(_) => Some(text("appId")?),
    };
    let launch = text("launch")?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    let listed = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or("manifest: `files` is missing")?;
    if listed.len() > MAX_FILES {
        return Err(format!("manifest: more than {MAX_FILES} files"));
    }
    for file in listed {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !valid_path(&path) {
            return Err(format!("manifest: file path {path:?} leaves its bundle"));
        }
        if !seen.insert(path.clone()) {
            return Err(format!("manifest: {path} is listed twice"));
        }
        let sha256 = file
            .get("sha256")
            .and_then(Value::as_str)
            .and_then(hex32)
            .ok_or_else(|| format!("manifest: {path} has no SHA-256"))?;
        let size = file
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("manifest: {path} has no size"))?;
        files.push(FileEntry { path, sha256, size });
    }
    if !seen.contains(&launch) {
        return Err(format!(
            "manifest: the launch file {launch} is not among its files"
        ));
    }
    Ok(Manifest {
        id,
        created_at,
        app_id,
        runtime: text("runtimeVersion")?,
        platform: text("platform")?,
        launch,
        files,
    })
}

/// The manifest's exact bytes, signed with the publisher's key.
pub fn verify(public_key: &[u8; 32], manifest: &[u8], signature: &str) -> Result<(), String> {
    let key = VerifyingKey::from_bytes(public_key)
        .map_err(|_| "the app's public key is invalid".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| "the signature is not base64".to_string())?;
    let signature = Signature::from_slice(&bytes)
        .map_err(|_| "the signature is not an Ed25519 signature".to_string())?;
    key.verify(manifest, &signature)
        .map_err(|_| "the manifest's signature does not match the app's key".to_string())
}

#[derive(Clone, Debug, Default, PartialEq)]
struct State {
    current: Option<String>,
    previous: Option<String>,
    pending: Option<String>,
    /// Launched and not yet ready: abandoned if the next start still finds it.
    trial: Option<String>,
    /// Updates that crashed on trial, newest last.
    failed: Vec<String>,
    /// The highest `createdAt` this phone ever accepted: an update is staged
    /// only when newer. A rollback after a crash does not lower it.
    newest: u64,
}

const KEPT_FAILURES: usize = 20;

impl State {
    fn read(path: &Path) -> State {
        let Ok(bytes) = std::fs::read(path) else {
            return State::default();
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return State::default();
        };
        let id = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| valid_id(s))
                .map(str::to_string)
        };
        State {
            current: id("current"),
            previous: id("previous"),
            pending: id("pending"),
            trial: id("trial"),
            failed: value
                .get("failed")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|s| valid_id(s))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            newest: value.get("newest").and_then(Value::as_u64).unwrap_or(0),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "current": self.current,
            "previous": self.previous,
            "pending": self.pending,
            "trial": self.trial,
            "failed": self.failed,
            "newest": self.newest,
        })
    }
}

/// An update whose manifest was accepted, receiving its files.
struct Staging {
    manifest: Manifest,
    bytes: Vec<u8>,
    received: HashSet<String>,
}

/// What `begin` decided: the update's id, and whether its files are wanted
/// (false when it is already current or pending).
#[derive(Debug, PartialEq)]
pub struct Begun {
    pub id: String,
    pub staging: bool,
}

/// An app's updates, in its private directory.
pub struct Store {
    dir: PathBuf,
    config: Config,
    state: State,
    staging: Option<Staging>,
}

impl Store {
    pub fn open(dir: &Path, config: Config) -> Result<Store, String> {
        std::fs::create_dir_all(dir.join("bundles"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let state = State::read(&dir.join("state.json"));
        let mut store = Store {
            dir: dir.to_path_buf(),
            config,
            state,
            staging: None,
        };
        if store.state.newest == 0 {
            // A state written before updates were ordered: the bar is what
            // this phone holds now.
            store.state.newest = [&store.state.current, &store.state.pending]
                .into_iter()
                .flatten()
                .filter_map(|id| store.manifest_of(id))
                .map(|m| m.created_at)
                .max()
                .unwrap_or(0);
        }
        Ok(store)
    }

    fn manifest_of(&self, id: &str) -> Option<Manifest> {
        parse_manifest(&std::fs::read(self.bundle(id).join("manifest.json")).ok()?).ok()
    }

    /// No update older than this is staged: the newest this phone accepted,
    /// or the bundle inside the app when that is newer.
    fn floor(&self) -> u64 {
        self.state
            .newest
            .max(self.config.embedded_created_at.unwrap_or(0))
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    fn save(&self) -> Result<(), String> {
        let tmp = self.dir.join("state.json.tmp");
        std::fs::write(&tmp, self.state.to_json().to_string())
            .map_err(|e| format!("state: {e}"))?;
        std::fs::rename(&tmp, self.dir.join("state.json")).map_err(|e| format!("state: {e}"))
    }

    fn bundle(&self, id: &str) -> PathBuf {
        self.dir.join("bundles").join(id)
    }

    /// The file to launch for bundle `id`, if it is there, still a manifest
    /// this runtime can run, and newer than the bundle inside the app.
    fn launch_path(&self, id: &str) -> Option<PathBuf> {
        let manifest = self.manifest_of(id)?;
        if manifest.runtime != self.config.runtime {
            return None;
        }
        if manifest.created_at <= self.config.embedded_created_at.unwrap_or(0) {
            // A store release brought a newer bundle than this update.
            return None;
        }
        let path = self.bundle(id).join("files").join(&manifest.launch);
        path.is_file().then_some(path)
    }

    /// At the app's start: roll back a trial that never got ready, promote a
    /// pending update. The bundle to launch, or None for the one inside the
    /// app.
    pub fn launch(&mut self) -> Result<Option<PathBuf>, String> {
        let s = &mut self.state;
        if let Some(trial) = s.trial.take() {
            // The last start launched it and it never reported ready.
            s.failed.push(trial.clone());
            if s.current.as_deref() == Some(trial.as_str()) {
                s.current = s.previous.take();
            }
            let _ = std::fs::remove_dir_all(self.dir.join("bundles").join(&trial));
        }
        if let Some(pending) = s.pending.take() {
            if s.current.as_deref() != Some(pending.as_str()) {
                s.previous = s.current.take();
                s.current = Some(pending.clone());
                s.trial = Some(pending);
            }
        }
        let excess = s.failed.len().saturating_sub(KEPT_FAILURES);
        s.failed.drain(..excess);
        // A bundle that vanished, no longer fits this runtime or is older
        // than the bundle inside the app (the app was updated from its store)
        // is dropped, and the one inside the app runs.
        let mut chosen = None;
        while let Some(id) = self.state.current.clone() {
            if let Some(path) = self.launch_path(&id) {
                chosen = Some(path);
                break;
            }
            self.state.current = self.state.previous.take();
            if self.state.trial.as_deref() == Some(id.as_str()) {
                self.state.trial = None;
            }
        }
        self.save()?;
        Ok(chosen)
    }

    /// The app showed its first screen on the current bundle.
    pub fn ready(&mut self) -> Result<(), String> {
        if self.state.trial.is_some() && self.state.trial == self.state.current {
            self.state.trial = None;
        }
        self.save()?;
        self.prune();
        Ok(())
    }

    /// Bundles no state refers to (an update being received is kept).
    fn prune(&self) {
        let receiving = self
            .staging
            .as_ref()
            .map(|s| format!("{}.staging", s.manifest.id));
        let keep: HashSet<&str> = [
            &self.state.current,
            &self.state.previous,
            &self.state.pending,
            &receiving,
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
        let Ok(entries) = std::fs::read_dir(self.dir.join("bundles")) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !keep.contains(name.as_str()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Everything a manifest must pass before anything it names is fetched:
    /// the signature first, then the runtime, platform and app, the crash
    /// record, and the order — an update not newer than the newest this
    /// phone accepted (or than the bundle inside the app) is refused, so a
    /// server or CDN replaying an old, validly signed update cannot roll the
    /// phone back. A publisher rolls back on purpose by publishing the old
    /// code again, which signs it with a new `createdAt`.
    fn check(&self, manifest_bytes: &[u8], signature: &str) -> Result<Manifest, String> {
        if manifest_bytes.len() > MAX_MANIFEST_BYTES {
            return Err(format!(
                "the manifest is larger than {MAX_MANIFEST_BYTES} bytes"
            ));
        }
        verify(&self.config.public_key, manifest_bytes, signature)?;
        let manifest = parse_manifest(manifest_bytes)?;
        if manifest.runtime != self.config.runtime {
            return Err(format!(
                "update {} needs runtime {}, this app is {}: it needs a new build from the store",
                manifest.id, manifest.runtime, self.config.runtime
            ));
        }
        if manifest.platform != self.config.platform {
            return Err(format!(
                "update {} is for {}, not {}",
                manifest.id, manifest.platform, self.config.platform
            ));
        }
        if let Some(app) = &self.config.app_id {
            if manifest.app_id.as_ref() != Some(app) {
                return Err(format!(
                    "update {} is for {}, not this app ({app})",
                    manifest.id,
                    manifest.app_id.as_deref().unwrap_or("no named app")
                ));
            }
        }
        if self.state.failed.contains(&manifest.id) {
            return Err(format!(
                "update {} crashed here before: not staged again",
                manifest.id
            ));
        }
        if self.is_held(&manifest.id) {
            return Ok(manifest);
        }
        let floor = self.floor();
        if manifest.created_at <= floor {
            return Err(format!(
                "E_ROBLEDO_NOT_NEWER: update {} was built at {}, not after {floor}, the newest this phone accepted",
                manifest.id, manifest.created_at
            ));
        }
        Ok(manifest)
    }

    fn is_held(&self, id: &str) -> bool {
        self.state.current.as_deref() == Some(id) || self.state.pending.as_deref() == Some(id)
    }

    /// Check a manifest and its signature (see `check`) and, when its files
    /// are wanted, start receiving them. Nothing is fetched before this.
    pub fn begin(&mut self, manifest_bytes: &[u8], signature: &str) -> Result<Begun, String> {
        self.staging = None;
        let manifest = self.check(manifest_bytes, signature)?;
        if self.is_held(&manifest.id) {
            return Ok(Begun {
                id: manifest.id,
                staging: false,
            });
        }
        let staging = self.staging_dir(&manifest.id);
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(staging.join("files")).map_err(|e| format!("staging: {e}"))?;
        let id = manifest.id.clone();
        self.staging = Some(Staging {
            manifest,
            bytes: manifest_bytes.to_vec(),
            received: HashSet::new(),
        });
        Ok(Begun { id, staging: true })
    }

    fn staging_dir(&self, id: &str) -> PathBuf {
        self.dir.join("bundles").join(format!("{id}.staging"))
    }

    /// One file of the update `begin` accepted: listed, not given twice, its
    /// size and digest those of the signed manifest. Written to disk at once.
    pub fn receive(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let staging = self
            .staging
            .as_mut()
            .ok_or("no update is being received: check its manifest first")?;
        let entry = staging
            .manifest
            .files
            .iter()
            .find(|f| f.path == path)
            .ok_or_else(|| format!("{path} is not a file the manifest lists"))?;
        if staging.received.contains(path) {
            return Err(format!("{path} was given twice"));
        }
        if data.len() as u64 != entry.size || Sha256::digest(data).as_slice() != entry.sha256 {
            return Err(format!("{path} does not match its digest in the manifest"));
        }
        let target = self
            .dir
            .join("bundles")
            .join(format!("{}.staging", staging.manifest.id))
            .join("files")
            .join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(&target, data).map_err(|e| format!("{path}: {e}"))?;
        staging.received.insert(path.to_string());
        Ok(())
    }

    /// Every file arrived: keep the update for the next start. Returns its id.
    pub fn finish(&mut self) -> Result<String, String> {
        let staging = self
            .staging
            .take()
            .ok_or("no update is being received: check its manifest first")?;
        if staging.received.len() != staging.manifest.files.len() {
            return Err("the files given are not the files the manifest lists".into());
        }
        let id = staging.manifest.id;
        let dir = self.staging_dir(&id);
        std::fs::write(dir.join("manifest.json"), &staging.bytes)
            .map_err(|e| format!("manifest: {e}"))?;
        let bundle = self.bundle(&id);
        let _ = std::fs::remove_dir_all(&bundle);
        std::fs::rename(&dir, &bundle).map_err(|e| format!("{}: {e}", bundle.display()))?;
        if let Some(old) = self.state.pending.replace(id.clone()) {
            let _ = std::fs::remove_dir_all(self.bundle(&old));
        }
        self.state.newest = self.state.newest.max(staging.manifest.created_at);
        self.save()?;
        Ok(id)
    }

    /// Verify and keep an update for the next start, all its files at hand.
    /// Returns its id.
    pub fn stage(
        &mut self,
        manifest_bytes: &[u8],
        signature: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<String, String> {
        let begun = self.begin(manifest_bytes, signature)?;
        if !begun.staging {
            return Ok(begun.id);
        }
        let given: HashSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        let listed: HashSet<&str> = self
            .staging
            .iter()
            .flat_map(|s| s.manifest.files.iter().map(|f| f.path.as_str()))
            .collect();
        let received = if given != listed || given.len() != files.len() {
            Err("the files given are not the files the manifest lists".to_string())
        } else {
            files
                .iter()
                .try_for_each(|(path, data)| self.receive(path, data))
        };
        if let Err(e) = received {
            self.staging = None;
            let _ = std::fs::remove_dir_all(self.staging_dir(&begun.id));
            return Err(e);
        }
        self.finish()
    }

    /// What JS reads: the app's runtime and server, and where updates stand.
    pub fn status(&self) -> Value {
        json!({
            "url": self.config.url,
            "runtimeVersion": self.config.runtime,
            "platform": self.config.platform,
            "current": self.state.current,
            "pending": self.state.pending,
            "trial": self.state.trial,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn config() -> Config {
        Config {
            url: "https://example.com/__robledo".into(),
            public_key: key().verifying_key().to_bytes(),
            runtime: "rt1".into(),
            platform: "android".into(),
            embedded_created_at: None,
            app_id: None,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("robledo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    // Fixed vectors: a signature or digest that changes across a crypto crate
    // upgrade would make every installed app refuse the next update.
    const RFC8032_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const RFC8032_SIGNATURE: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
    const VECTOR_MESSAGE: &[u8] = b"robledo update manifest vector";
    /// The public key, signature and digest of VECTOR_MESSAGE under the `[7; 32]`
    /// seed, as ed25519-dalek 2 / sha2 0.10 / base64 0.22 produced them.
    const VECTOR_PUBLIC: &str = "6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=";
    const VECTOR_SIGNATURE: &str =
        "bkEEfchf5aF/atZ+tO0U5Tb254nZNLunaeSIx5YjjzJ3iLVS7RNsDPHUsBb9acM/nIFn3H81Uc12xZl2V3MzDg==";
    const VECTOR_SHA256: &str = "bb558155f17e9a932b286dd54c5d5e143ff8ab009824b1a2dfdb019895e4c961";

    #[test]
    fn verifies_rfc8032_test_1() {
        let public = hex32(RFC8032_PUBLIC).expect("hex");
        let signature = (0..64)
            .map(|i| u8::from_str_radix(&RFC8032_SIGNATURE[2 * i..2 * i + 2], 16).expect("hex"))
            .collect::<Vec<u8>>();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&signature);
        assert_eq!(verify(&public, b"", &encoded), Ok(()));
        assert!(verify(&public, b"x", &encoded).is_err());
    }

    #[test]
    fn keeps_the_signatures_and_digests_of_earlier_releases() {
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(key().verifying_key().to_bytes()),
            VECTOR_PUBLIC
        );
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(key().sign(VECTOR_MESSAGE).to_bytes()),
            VECTOR_SIGNATURE
        );
        assert_eq!(
            verify(
                &key().verifying_key().to_bytes(),
                VECTOR_MESSAGE,
                VECTOR_SIGNATURE
            ),
            Ok(())
        );
        assert_eq!(hex(&Sha256::digest(VECTOR_MESSAGE)), VECTOR_SHA256);
    }

    /// A signed update `id` whose bundle prints `body`.
    type Files = Vec<(String, Vec<u8>)>;

    /// Each update built after the one before, as `ream android:publish` stamps them.
    static CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_767_225_600);

    fn update(id: &str, body: &str, runtime: &str) -> (Vec<u8>, String, Files) {
        let created = CLOCK.fetch_add(60, std::sync::atomic::Ordering::Relaxed);
        update_with(id, body, runtime, json!({ "createdAt": created }))
    }

    /// An update whose manifest carries `extra` (`createdAt`, `appId`).
    fn update_with(id: &str, body: &str, runtime: &str, extra: Value) -> (Vec<u8>, String, Files) {
        let bundle = body.as_bytes().to_vec();
        let mut manifest = json!({
            "id": id, "runtimeVersion": runtime, "platform": "android",
            "launch": "index.android.bundle",
            "files": [{ "path": "index.android.bundle", "sha256": hex(&Sha256::digest(&bundle)), "size": bundle.len() }],
        });
        if let (Some(m), Some(e)) = (manifest.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        let manifest = manifest.to_string().into_bytes();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(key().sign(&manifest).to_bytes());
        (
            manifest,
            signature,
            vec![("index.android.bundle".into(), bundle)],
        )
    }

    fn body(path: Option<PathBuf>) -> Option<String> {
        path.map(|p| std::fs::read_to_string(p).unwrap())
    }

    #[test]
    fn a_staged_update_launches_at_the_next_start_and_stays_once_ready() {
        let dir = temp("cycle");
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(
            store.launch().unwrap(),
            None,
            "nothing staged: the app's own bundle"
        );
        let (m, s, f) = update("u1", "one", "rt1");
        assert_eq!(store.stage(&m, &s, &f).unwrap(), "u1");
        // Reopened as at the next start: the state is on disk.
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("one"));
        store.ready().unwrap();
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("one"));
    }

    #[test]
    fn an_update_that_never_reports_ready_is_rolled_back_and_not_staged_again() {
        let dir = temp("rollback");
        let mut store = Store::open(&dir, config()).unwrap();
        let (m, s, f) = update("good", "good", "rt1");
        store.stage(&m, &s, &f).unwrap();
        store.launch().unwrap();
        store.ready().unwrap();
        let (m2, s2, f2) = update("bad", "bad", "rt1");
        store.stage(&m2, &s2, &f2).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("bad"));
        // It crashed before its first screen: no ready. The next start goes back.
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("good"));
        assert!(store.stage(&m2, &s2, &f2).unwrap_err().contains("crashed"));
        assert!(!dir.join("bundles/bad").exists());
    }

    #[test]
    fn a_first_update_that_crashes_falls_back_to_the_bundle_inside_the_app() {
        let dir = temp("first");
        let mut store = Store::open(&dir, config()).unwrap();
        let (m, s, f) = update("u1", "one", "rt1");
        store.stage(&m, &s, &f).unwrap();
        assert!(store.launch().unwrap().is_some());
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(store.launch().unwrap(), None);
    }

    #[test]
    fn refuses_what_was_not_signed_by_the_apps_key_or_was_altered() {
        let dir = temp("refuse");
        let mut store = Store::open(&dir, config()).unwrap();
        let (m, s, f) = update("u1", "one", "rt1");
        // Another key.
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let forged = base64::engine::general_purpose::STANDARD.encode(other.sign(&m).to_bytes());
        assert!(store
            .stage(&m, &forged, &f)
            .unwrap_err()
            .contains("signature"));
        // A manifest changed after signing.
        let mut altered = m.clone();
        let last = altered.len() - 2;
        altered[last] = b' ';
        assert!(store.stage(&altered, &s, &f).is_err());
        // A file changed in transit.
        let tampered = vec![("index.android.bundle".to_string(), b"evil".to_vec())];
        assert!(store
            .stage(&m, &s, &tampered)
            .unwrap_err()
            .contains("digest"));
        // A file missing, or one more.
        assert!(store
            .stage(&m, &s, &[])
            .unwrap_err()
            .contains("not the files"));
        let mut extra = f.clone();
        extra.push(("x.js".into(), vec![]));
        assert!(store
            .stage(&m, &s, &extra)
            .unwrap_err()
            .contains("not the files"));
        // Another runtime: needs a store build.
        let (m3, s3, f3) = update("u3", "three", "rt2");
        assert!(store.stage(&m3, &s3, &f3).unwrap_err().contains("runtime"));
        // Nothing was staged by any of it.
        assert_eq!(store.launch().unwrap(), None);
    }

    #[test]
    fn a_manifest_cannot_name_a_path_outside_its_bundle() {
        for path in ["../x", "/etc/passwd", "a/../../b", "a\\b", ""] {
            assert!(!valid_path(path), "{path}");
        }
        assert!(valid_path("assets/img/logo.png"));
        let manifest = json!({ "id": "u", "createdAt": 1, "runtimeVersion": "rt1", "platform": "android", "launch": "../x",
                               "files": [{ "path": "../x", "sha256": "0".repeat(64), "size": 0 }] });
        assert!(parse_manifest(manifest.to_string().as_bytes())
            .unwrap_err()
            .contains("leaves"));
        let bad_id = json!({ "id": "../u", "createdAt": 1, "runtimeVersion": "rt1", "platform": "android", "launch": "a", "files": [] });
        assert!(parse_manifest(bad_id.to_string().as_bytes())
            .unwrap_err()
            .contains("bundle id"));
    }

    #[test]
    fn an_app_updated_from_its_store_drops_bundles_of_the_old_runtime() {
        let dir = temp("runtime");
        let mut store = Store::open(&dir, config()).unwrap();
        let (m, s, f) = update("u1", "one", "rt1");
        store.stage(&m, &s, &f).unwrap();
        store.launch().unwrap();
        store.ready().unwrap();
        let newer = Config {
            runtime: "rt2".into(),
            ..config()
        };
        let mut store = Store::open(&dir, newer).unwrap();
        assert_eq!(store.launch().unwrap(), None);
    }

    #[test]
    fn reads_the_config_the_build_wrote() {
        let public =
            base64::engine::general_purpose::STANDARD.encode(key().verifying_key().to_bytes());
        let parsed = parse_config(
            json!({ "url": "https://x/__robledo", "publicKey": public, "runtimeVersion": "rt1", "platform": "android" })
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed, config_with_url("https://x/__robledo"));
        assert!(parse_config(
            br#"{"url":"u","publicKey":"AAAA","runtimeVersion":"r","platform":"android"}"#
        )
        .unwrap_err()
        .contains("32-byte"));
    }

    fn config_with_url(url: &str) -> Config {
        Config {
            url: url.into(),
            ..config()
        }
    }

    fn signed(manifest: &Value) -> (Vec<u8>, String) {
        let bytes = manifest.to_string().into_bytes();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(key().sign(&bytes).to_bytes());
        (bytes, signature)
    }

    #[test]
    fn an_older_signed_update_never_replaces_a_newer_one() {
        let dir = temp("replay");
        // 2026-01: the vulnerable release. 2026-06: the release that fixes it.
        let old = update_with(
            "1767225600-a",
            "vulnerable",
            "rt1",
            json!({ "createdAt": 1_767_225_600u64 }),
        );
        let new = update_with(
            "1780272000-b",
            "fixed",
            "rt1",
            json!({ "createdAt": 1_780_272_000u64 }),
        );
        let mut store = Store::open(&dir, config()).unwrap();
        store.stage(&old.0, &old.1, &old.2).unwrap();
        store.launch().unwrap();
        store.ready().unwrap();
        store.stage(&new.0, &new.1, &new.2).unwrap();
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("fixed"));
        store.ready().unwrap();
        // The server, or anything in front of it, answers with the old update.
        let refused = store.stage(&old.0, &old.1, &old.2).unwrap_err();
        assert!(refused.starts_with("E_ROBLEDO_NOT_NEWER: "), "{refused}");
        let begun = store.begin(&old.0, &old.1).unwrap_err();
        assert!(begun.starts_with("E_ROBLEDO_NOT_NEWER: "), "{begun}");
        // Built in the same second: not newer either.
        let same = update_with(
            "1780272000-c",
            "same second",
            "rt1",
            json!({ "createdAt": 1_780_272_000u64 }),
        );
        assert!(store.stage(&same.0, &same.1, &same.2).is_err());
        // The next start still runs the fix.
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("fixed"));
        // A deliberate rollback: the old code published again, so signed anew.
        let again = update_with(
            "1790000000-a",
            "vulnerable",
            "rt1",
            json!({ "createdAt": 1_790_000_000u64 }),
        );
        store.ready().unwrap();
        assert_eq!(
            store.stage(&again.0, &again.1, &again.2).unwrap(),
            "1790000000-a"
        );
    }

    #[test]
    fn a_rollback_after_a_crash_does_not_lower_the_bar() {
        let dir = temp("bar");
        let good = update_with("good", "good", "rt1", json!({ "createdAt": 100 }));
        let bad = update_with("bad", "bad", "rt1", json!({ "createdAt": 200 }));
        let between = update_with("between", "between", "rt1", json!({ "createdAt": 150 }));
        let mut store = Store::open(&dir, config()).unwrap();
        store.stage(&good.0, &good.1, &good.2).unwrap();
        store.launch().unwrap();
        store.ready().unwrap();
        store.stage(&bad.0, &bad.1, &bad.2).unwrap();
        store.launch().unwrap();
        // It crashed: the next start goes back to `good`, the bar stays at 200.
        let mut store = Store::open(&dir, config()).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("good"));
        assert!(store
            .stage(&between.0, &between.1, &between.2)
            .unwrap_err()
            .starts_with("E_ROBLEDO_NOT_NEWER"));
        let state = std::fs::read_to_string(dir.join("state.json")).unwrap();
        assert!(state.contains(r#""newest":200"#), "{state}");
    }

    #[test]
    fn a_state_written_before_updates_were_ordered_takes_its_bar_from_what_it_holds() {
        let dir = temp("legacy");
        let held = update_with("held", "held", "rt1", json!({ "createdAt": 500 }));
        let older = update_with("older", "older", "rt1", json!({ "createdAt": 400 }));
        let mut store = Store::open(&dir, config()).unwrap();
        store.stage(&held.0, &held.1, &held.2).unwrap();
        store.launch().unwrap();
        store.ready().unwrap();
        // The state as robledo 0.1.1 wrote it: no `newest`.
        std::fs::write(
            dir.join("state.json"),
            r#"{"current":"held","previous":null,"pending":null,"trial":null,"failed":[]}"#,
        )
        .unwrap();
        let mut store = Store::open(&dir, config()).unwrap();
        assert!(store.stage(&older.0, &older.1, &older.2).is_err());
    }

    #[test]
    fn a_store_release_with_a_newer_bundle_wins_over_older_updates() {
        let dir = temp("store");
        let ota = update_with("ota", "ota", "rt1", json!({ "createdAt": 1000 }));
        let mut store = Store::open(&dir, config()).unwrap();
        store.stage(&ota.0, &ota.1, &ota.2).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("ota"));
        store.ready().unwrap();
        // The store release, same runtime, its bundle built after the update.
        let released = Config {
            embedded_created_at: Some(2000),
            ..config()
        };
        let mut store = Store::open(&dir, released.clone()).unwrap();
        assert_eq!(store.launch().unwrap(), None);
        // An update older than the bundle inside the app is refused…
        let stale = update_with("stale", "stale", "rt1", json!({ "createdAt": 1500 }));
        assert!(store
            .stage(&stale.0, &stale.1, &stale.2)
            .unwrap_err()
            .starts_with("E_ROBLEDO_NOT_NEWER"));
        // …a newer one is staged and launched.
        let next = update_with("next", "next", "rt1", json!({ "createdAt": 2500 }));
        store.stage(&next.0, &next.1, &next.2).unwrap();
        let mut store = Store::open(&dir, released).unwrap();
        assert_eq!(body(store.launch().unwrap()).as_deref(), Some("next"));
        // A build stamped `embeddedCreatedAt` and `appId` into its config.
        let public =
            base64::engine::general_purpose::STANDARD.encode(key().verifying_key().to_bytes());
        let parsed = parse_config(
            json!({ "url": "u", "publicKey": public, "runtimeVersion": "rt1", "platform": "android",
                    "embeddedCreatedAt": 2000, "appId": "ch.example.app" })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.embedded_created_at, Some(2000));
        assert_eq!(parsed.app_id.as_deref(), Some("ch.example.app"));
        assert!(parse_config(
            json!({ "url": "u", "publicKey": public, "runtimeVersion": "rt1", "platform": "android", "embeddedCreatedAt": "x" })
                .to_string()
                .as_bytes()
        )
        .is_err());
    }

    #[test]
    fn an_app_that_knows_its_id_refuses_another_apps_update() {
        let dir = temp("app");
        let config = Config {
            app_id: Some("ch.example.one".into()),
            ..config()
        };
        let mut store = Store::open(&dir, config).unwrap();
        let other = update_with(
            "o",
            "o",
            "rt1",
            json!({ "createdAt": 10, "appId": "ch.example.two" }),
        );
        assert!(store
            .stage(&other.0, &other.1, &other.2)
            .unwrap_err()
            .contains("not this app"));
        let unnamed = update_with("u", "u", "rt1", json!({ "createdAt": 10 }));
        assert!(store
            .stage(&unnamed.0, &unnamed.1, &unnamed.2)
            .unwrap_err()
            .contains("no named app"));
        let mine = update_with(
            "m",
            "m",
            "rt1",
            json!({ "createdAt": 10, "appId": "ch.example.one" }),
        );
        assert_eq!(store.stage(&mine.0, &mine.1, &mine.2).unwrap(), "m");
    }

    #[test]
    fn files_are_received_one_by_one_only_after_the_manifest_holds() {
        let dir = temp("receive");
        let mut store = Store::open(&dir, config()).unwrap();
        assert!(store.receive("index.android.bundle", b"x").is_err());
        assert!(store.finish().is_err());
        let a = b"bundle".to_vec();
        let b = b"logo".to_vec();
        let (manifest, signature) = signed(&json!({
            "id": "two", "createdAt": 10, "runtimeVersion": "rt1", "platform": "android",
            "launch": "index.android.bundle",
            "files": [
                { "path": "index.android.bundle", "sha256": hex(&Sha256::digest(&a)), "size": a.len() },
                { "path": "assets/logo.png", "sha256": hex(&Sha256::digest(&b)), "size": b.len() },
            ],
        }));
        // A forged signature: refused before any file.
        assert!(store.begin(&manifest, "AAAA").is_err());
        assert!(store.receive("index.android.bundle", &a).is_err());
        let begun = store.begin(&manifest, &signature).unwrap();
        assert_eq!(
            begun,
            Begun {
                id: "two".into(),
                staging: true
            }
        );
        assert!(store
            .receive("other.js", &a)
            .unwrap_err()
            .contains("not a file"));
        assert!(store
            .receive("index.android.bundle", b"bundlE")
            .unwrap_err()
            .contains("digest"));
        assert!(store
            .receive("index.android.bundle", b"bundle and more")
            .unwrap_err()
            .contains("digest"));
        store.receive("index.android.bundle", &a).unwrap();
        assert!(store
            .receive("index.android.bundle", &a)
            .unwrap_err()
            .contains("twice"));
        assert!(store.finish().unwrap_err().contains("not the files"));
        // Begin again: the files are received anew, and `ready` meanwhile
        // (the app reaching its first screen) keeps what arrived.
        store.begin(&manifest, &signature).unwrap();
        store.receive("index.android.bundle", &a).unwrap();
        store.ready().unwrap();
        assert!(dir
            .join("bundles/two.staging/files/index.android.bundle")
            .is_file());
        store.receive("assets/logo.png", &b).unwrap();
        assert_eq!(store.finish().unwrap(), "two");
        assert!(dir.join("bundles/two/files/assets/logo.png").is_file());
        // Already pending: nothing more is wanted.
        assert_eq!(
            store.begin(&manifest, &signature).unwrap(),
            Begun {
                id: "two".into(),
                staging: false
            }
        );
    }

    #[test]
    fn a_manifest_is_bounded_before_it_is_read() {
        let dir = temp("bounds");
        let mut store = Store::open(&dir, config()).unwrap();
        let huge = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(store.begin(&huge, "AAAA").unwrap_err().contains("larger"));
        let files: Vec<Value> = (0..=MAX_FILES)
            .map(|i| json!({ "path": format!("f/{i}"), "sha256": "0".repeat(64), "size": 0 }))
            .collect();
        let (manifest, signature) = signed(&json!({
            "id": "many", "createdAt": 10, "runtimeVersion": "rt1", "platform": "android",
            "launch": "f/0", "files": files,
        }));
        assert!(store
            .begin(&manifest, &signature)
            .unwrap_err()
            .contains("more than"));
        let (manifest, signature) = signed(&json!({
            "id": "undated", "runtimeVersion": "rt1", "platform": "android", "launch": "a",
            "files": [{ "path": "a", "sha256": "0".repeat(64), "size": 0 }],
        }));
        assert!(store
            .begin(&manifest, &signature)
            .unwrap_err()
            .contains("createdAt"));
    }
}
