//! robledo's client, on the phone.
//!
//! An update is a manifest — its id, the native runtime it needs, the file to
//! launch, and every file with its SHA-256 and size — signed with Ed25519 by
//! `ream android:publish`, and the files it lists. This crate:
//!
//! - **stages** one: checks the signature against the public key built into
//!   the app, the runtime against the app's own, each file against its
//!   digest, and writes it under `bundles/<id>` beside a `pending` mark —
//!   nothing a network or a server could alter is ever launched;
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

/// What the app was built with (`ream android:*` writes it into the app).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub url: String,
    pub public_key: [u8; 32],
    pub runtime: String,
    pub platform: String,
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
    Ok(Config {
        url: text("url")?,
        public_key,
        runtime: text("runtimeVersion")?,
        platform: text("platform")?,
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
    let launch = text("launch")?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for file in value
        .get("files")
        .and_then(Value::as_array)
        .ok_or("manifest: `files` is missing")?
    {
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
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "current": self.current,
            "previous": self.previous,
            "pending": self.pending,
            "trial": self.trial,
            "failed": self.failed,
        })
    }
}

/// An app's updates, in its private directory.
pub struct Store {
    dir: PathBuf,
    config: Config,
    state: State,
}

impl Store {
    pub fn open(dir: &Path, config: Config) -> Result<Store, String> {
        std::fs::create_dir_all(dir.join("bundles"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let state = State::read(&dir.join("state.json"));
        Ok(Store {
            dir: dir.to_path_buf(),
            config,
            state,
        })
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

    /// The file to launch for bundle `id`, if it is there and still a
    /// manifest this runtime can run.
    fn launch_path(&self, id: &str) -> Option<PathBuf> {
        let manifest =
            parse_manifest(&std::fs::read(self.bundle(id).join("manifest.json")).ok()?).ok()?;
        if manifest.runtime != self.config.runtime {
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
        // A bundle that vanished or no longer fits this runtime (the app was
        // updated from its store) is dropped, and the one inside the app runs.
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

    /// Bundles no state refers to.
    fn prune(&self) {
        let keep: HashSet<&str> = [
            &self.state.current,
            &self.state.previous,
            &self.state.pending,
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

    /// Verify and keep an update for the next start. Returns its id.
    pub fn stage(
        &mut self,
        manifest_bytes: &[u8],
        signature: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<String, String> {
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
        if self.state.failed.contains(&manifest.id) {
            return Err(format!(
                "update {} crashed here before: not staged again",
                manifest.id
            ));
        }
        if self.state.current.as_deref() == Some(manifest.id.as_str())
            || self.state.pending.as_deref() == Some(manifest.id.as_str())
        {
            return Ok(manifest.id);
        }
        let given: HashSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        let listed: HashSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        if given != listed || given.len() != files.len() {
            return Err("the files given are not the files the manifest lists".into());
        }
        for entry in &manifest.files {
            let data = files
                .iter()
                .find(|(p, _)| *p == entry.path)
                .map(|(_, d)| d)
                .ok_or("missing file")?;
            if data.len() as u64 != entry.size || Sha256::digest(data).as_slice() != entry.sha256 {
                return Err(format!(
                    "{} does not match its digest in the manifest",
                    entry.path
                ));
            }
        }
        let staging = self
            .dir
            .join("bundles")
            .join(format!("{}.staging", manifest.id));
        let _ = std::fs::remove_dir_all(&staging);
        for (path, data) in files {
            let target = staging.join("files").join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{path}: {e}"))?;
            }
            std::fs::write(&target, data).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(staging.join("manifest.json"), manifest_bytes)
            .map_err(|e| format!("manifest: {e}"))?;
        let bundle = self.bundle(&manifest.id);
        let _ = std::fs::remove_dir_all(&bundle);
        std::fs::rename(&staging, &bundle).map_err(|e| format!("{}: {e}", bundle.display()))?;
        if let Some(old) = self.state.pending.replace(manifest.id.clone()) {
            let _ = std::fs::remove_dir_all(self.bundle(&old));
        }
        self.save()?;
        Ok(manifest.id)
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

    /// A signed update `id` whose bundle prints `body`.
    type Files = Vec<(String, Vec<u8>)>;

    fn update(id: &str, body: &str, runtime: &str) -> (Vec<u8>, String, Files) {
        let bundle = body.as_bytes().to_vec();
        let manifest = json!({
            "id": id, "runtimeVersion": runtime, "platform": "android",
            "launch": "index.android.bundle",
            "files": [{ "path": "index.android.bundle", "sha256": hex(&Sha256::digest(&bundle)), "size": bundle.len() }],
        })
        .to_string()
        .into_bytes();
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
        let manifest = json!({ "id": "u", "runtimeVersion": "rt1", "platform": "android", "launch": "../x",
                               "files": [{ "path": "../x", "sha256": "0".repeat(64), "size": 0 }] });
        assert!(parse_manifest(manifest.to_string().as_bytes())
            .unwrap_err()
            .contains("leaves"));
        let bad_id = json!({ "id": "../u", "runtimeVersion": "rt1", "platform": "android", "launch": "a", "files": [] });
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
}
