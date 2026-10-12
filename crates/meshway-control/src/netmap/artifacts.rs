//! Rule-set artifacts of the held netmap. Each referenced digest is fetched
//! once from `GET /agent-api/v1/artifacts/{digest}`, checked (length, SHA-256,
//! full `MWRS` structure) and kept as an owner read-only file
//! `artifacts/sha256-<hex>.mwrs` beside `netmap.json`, published by atomic
//! rename and read (and verified again) once per process. Files the held netmap no longer
//! references are removed. Artifacts are content, never authority: a missing
//! one leaves the rules naming it unavailable (Policy), it grants nothing.
use crate::{Control, Error};
use meshway_configuration::netmap::RuleSetRef;
use meshway_control_client::{Client, ClientError, GetRuleSetArtifactPath};
use meshway_policy::compile::Artifacts;
use meshway_private_storage::Directory;
use meshway_protocol::ErrorResponse;
use meshway_rules::{Digest, RuleSet};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(crate) const DIRECTORY: &str = "artifacts";
const LOCK: &str = "artifacts.lock";
/// Downloads in flight at once.
const PARALLEL: usize = 2;
/// Every artifact one netmap references, together.
pub(crate) const MAX_TOTAL_BYTES: u64 = 64 << 20;
const RETRY_MIN: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

pub use meshway_protocol::ipc::ArtifactProblem;

#[derive(Default)]
pub(crate) struct State {
    directory: Option<Arc<Directory>>,
    loaded: Artifacts,
    /// `loaded` restricted to the held netmap, as published with it.
    published: Arc<Artifacts>,
    failures: BTreeMap<Digest, Failure>,
    last_problem: Option<ArtifactProblem>,
}

struct Failure {
    attempts: u32,
    retry_at: Instant,
}

/// Downloads prepared under the Control driver, sent without it.
pub struct ArtifactCall {
    client: Client,
    token: Zeroizing<String>,
    references: Vec<RuleSetRef>,
}

type Fetched = (RuleSetRef, Result<Vec<u8>, ClientError<ErrorResponse>>);

pub struct ArtifactReply {
    results: Vec<Fetched>,
}

impl ArtifactCall {
    pub async fn send(self) -> ArtifactReply {
        let fetch = |reference: RuleSetRef| {
            let (client, token) = (&self.client, &self.token);
            async move {
                let path = GetRuleSetArtifactPath {
                    digest: reference.digest.to_string(),
                };
                let result = client.get_rule_set_artifact(&path, token).await;
                (reference, result)
            }
        };
        ArtifactReply {
            results: futures_util::future::join_all(self.references.iter().cloned().map(fetch))
                .await,
        }
    }
}

fn verify(reference: &RuleSetRef, bytes: Vec<u8>) -> Result<RuleSet, ArtifactProblem> {
    if bytes.len() as u64 != u64::from(reference.bytes) || Digest::of(&bytes) != reference.digest {
        return Err(ArtifactProblem::Integrity);
    }
    let set = RuleSet::from_vec(bytes).map_err(|_| ArtifactProblem::Integrity)?;
    if set.counts().entries() != u64::from(reference.entries) {
        return Err(ArtifactProblem::Integrity);
    }
    Ok(set)
}

fn load_cached(
    directory: &Directory,
    wanted: Vec<RuleSetRef>,
    keep: &[String],
) -> Vec<(Digest, Arc<RuleSet>)> {
    let mut loaded = Vec::new();
    for reference in wanted {
        match open_cached(directory, &reference) {
            Ok(Some(set)) => loaded.push((reference.digest, Arc::new(set))),
            Ok(None) => {}
            Err(()) => {
                let _ = directory.remove_read_only(&reference.digest.file_name());
            }
        }
    }
    if let Ok(names) = directory.names() {
        for name in names.iter().filter(|name| !keep.contains(name)) {
            let removed = if meshway_private_storage::is_staging(name) {
                directory.remove_staging(name)
            } else {
                directory.remove_read_only(name)
            };
            if let Err(error) = removed {
                tracing::warn!(file = %name, %error, "Artifact cache file could not be removed");
            }
        }
    }
    loaded
}

/// Remove every cached artifact (the Node left its network).
pub(crate) fn forget(control_directory: &Directory) -> Result<(), meshway_private_storage::Error> {
    let Some(directory) =
        Directory::open_existing(&control_directory.path().join(DIRECTORY), LOCK)?
    else {
        return Ok(());
    };
    load_cached(&directory, Vec::new(), &[]);
    Ok(())
}

/// The cached file of `reference`, read and checked; `None` when absent.
fn open_cached(directory: &Directory, reference: &RuleSetRef) -> Result<Option<RuleSet>, ()> {
    use std::io::Read;
    let Some(file) = directory
        .read_only_file(&reference.digest.file_name())
        .map_err(|_| ())?
    else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(meshway_rules::MAX_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    let set = RuleSet::from_vec(bytes).map_err(|_| ())?;
    (set.digest() == reference.digest
        && set.as_bytes().len() as u64 == u64::from(reference.bytes)
        && set.counts().entries() == u64::from(reference.entries))
    .then_some(Some(set))
    .ok_or(())
}

impl Control {
    fn artifact_references(&self) -> BTreeMap<Digest, RuleSetRef> {
        self.netmap
            .base
            .as_ref()
            .map(|base| {
                base.netmap
                    .projection
                    .rule_sets
                    .values()
                    .map(|reference| (reference.digest, reference.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn artifact_directory(&mut self) -> Result<Arc<Directory>, Error> {
        if let Some(directory) = &self.netmap.artifacts.directory {
            return Ok(Arc::clone(directory));
        }
        let directory = Arc::new(Directory::open(
            &self.directory.path().join(DIRECTORY),
            LOCK,
        )?);
        self.netmap.artifacts.directory = Some(Arc::clone(&directory));
        Ok(directory)
    }

    /// The loaded artifacts the held netmap references.
    pub(crate) fn published_artifacts(&self) -> Arc<Artifacts> {
        Arc::clone(&self.netmap.artifacts.published)
    }

    fn publish_artifacts(&mut self) {
        let references = self.artifact_references();
        let state = &mut self.netmap.artifacts;
        state
            .loaded
            .retain(|digest, _| references.contains_key(digest));
        state
            .failures
            .retain(|digest, _| references.contains_key(digest));
        if state.loaded.keys().eq(state.published.keys()) {
            return;
        }
        state.published = Arc::new(state.loaded.clone());
    }

    fn cache_plan(&mut self) -> Option<(Arc<Directory>, Vec<RuleSetRef>, Vec<String>)> {
        let references = self.artifact_references();
        let directory = match self.artifact_directory() {
            Ok(directory) => directory,
            Err(_) => {
                self.netmap.artifacts.last_problem = Some(ArtifactProblem::Storage);
                self.publish_artifacts();
                return None;
            }
        };
        let wanted = references
            .values()
            .filter(|reference| !self.netmap.artifacts.loaded.contains_key(&reference.digest))
            .cloned()
            .collect();
        let keep = references.keys().map(Digest::file_name).collect();
        Some((directory, wanted, keep))
    }

    /// Load the cached artifacts the held netmap references and remove every
    /// other file. A cached file that does not verify is removed and fetched
    /// again. Storage trouble only leaves artifacts missing.
    pub(crate) async fn load_artifacts(&mut self) {
        if let Some((directory, wanted, keep)) = self.cache_plan() {
            let loaded =
                tokio::task::spawn_blocking(move || load_cached(&directory, wanted, &keep))
                    .await
                    .unwrap_or_default();
            self.netmap.artifacts.loaded.extend(loaded);
        }
        self.publish_artifacts();
    }

    /// [`Self::load_artifacts`] on the caller's thread (synchronous opening).
    pub(crate) fn load_artifacts_blocking(&mut self) {
        if let Some((directory, wanted, keep)) = self.cache_plan() {
            let loaded = load_cached(&directory, wanted, &keep);
            self.netmap.artifacts.loaded.extend(loaded);
        }
        self.publish_artifacts();
    }

    /// Referenced artifacts not loaded, in digest order.
    pub fn missing_artifacts(&self) -> Vec<Digest> {
        self.artifact_references()
            .into_keys()
            .filter(|digest| !self.netmap.artifacts.loaded.contains_key(digest))
            .collect()
    }

    /// The last fetch or storage failure while some artifact is missing.
    pub fn artifact_problem(&self) -> Option<ArtifactProblem> {
        self.netmap
            .artifacts
            .last_problem
            .filter(|_| !self.missing_artifacts().is_empty())
    }

    /// Up to two missing artifacts whose retry is due, with the session
    /// token; `None` when nothing is due or no session is live.
    pub fn begin_artifacts(&mut self) -> Option<ArtifactCall> {
        let token = Zeroizing::new(self.token().ok()?.to_owned());
        let now = Instant::now();
        let references = self.artifact_references();
        if references.values().map(|r| u64::from(r.bytes)).sum::<u64>() > MAX_TOTAL_BYTES {
            self.netmap.artifacts.last_problem = Some(ArtifactProblem::Budget);
            return None;
        }
        let due: Vec<RuleSetRef> = references
            .into_values()
            .filter(|reference| {
                !self.netmap.artifacts.loaded.contains_key(&reference.digest)
                    && self
                        .netmap
                        .artifacts
                        .failures
                        .get(&reference.digest)
                        .is_none_or(|failure| failure.retry_at <= now)
            })
            .take(PARALLEL)
            .collect();
        (!due.is_empty()).then(|| ArtifactCall {
            client: self.client.clone(),
            token,
            references: due,
        })
    }

    /// Verify, store and load fetched artifacts. Returns whether the
    /// published set changed. Failures back off from 5 s to 5 min.
    pub async fn finish_artifacts(&mut self, reply: ArtifactReply) -> bool {
        let references = self.artifact_references();
        let directory = self.artifact_directory().ok();
        let mut changed = false;
        for (reference, result) in reply.results {
            if !references.contains_key(&reference.digest)
                || self.netmap.artifacts.loaded.contains_key(&reference.digest)
            {
                continue;
            }
            let stored = match (result, &directory) {
                (Err(_), _) => Err(ArtifactProblem::Download),
                (Ok(_), None) => Err(ArtifactProblem::Storage),
                (Ok(bytes), Some(directory)) => {
                    let directory = Arc::clone(directory);
                    let reference = reference.clone();
                    tokio::task::spawn_blocking(move || {
                        let set = verify(&reference, bytes)?;
                        directory
                            .publish_read_only(&reference.digest.file_name(), set.as_bytes())
                            .map_err(|_| ArtifactProblem::Storage)?;
                        drop(set);
                        // The process keeps one read-only mapping per digest.
                        open_cached(&directory, &reference)
                            .ok()
                            .flatten()
                            .ok_or(ArtifactProblem::Storage)
                    })
                    .await
                    .unwrap_or(Err(ArtifactProblem::Storage))
                }
            };
            match stored {
                Ok(set) => {
                    self.netmap.artifacts.failures.remove(&reference.digest);
                    self.netmap
                        .artifacts
                        .loaded
                        .insert(reference.digest, Arc::new(set));
                    changed = true;
                }
                Err(problem) => {
                    tracing::warn!(
                        digest = %reference.digest,
                        ?problem,
                        "Rule-set artifact unavailable; its rules stay unavailable"
                    );
                    self.netmap.artifacts.last_problem = Some(problem);
                    let failure = self
                        .netmap
                        .artifacts
                        .failures
                        .entry(reference.digest)
                        .or_insert(Failure {
                            attempts: 0,
                            retry_at: Instant::now(),
                        });
                    failure.attempts = failure.attempts.saturating_add(1);
                    let delay = RETRY_MIN
                        .saturating_mul(1 << failure.attempts.saturating_sub(1).min(6))
                        .min(RETRY_MAX);
                    failure.retry_at = Instant::now() + delay;
                }
            }
        }
        if changed {
            self.publish_artifacts();
        }
        changed
    }
}

/// Fetch every due missing artifact of the held netmap, publishing the
/// netmap again each time one more is loaded. Holds the driver only to
/// prepare and apply.
pub async fn fetch_artifacts(
    control: &tokio::sync::Mutex<Control>,
    netmaps: &tokio::sync::watch::Sender<Option<meshway_engine::Netmap>>,
    stop: &tokio_util::sync::CancellationToken,
) {
    loop {
        let Some(call) = control.lock().await.begin_artifacts() else {
            return;
        };
        let reply = tokio::select! { biased;
            _ = stop.cancelled() => return,
            reply = call.send() => reply,
        };
        let mut driver = control.lock().await;
        if driver.finish_artifacts(reply).await
            && let Some(netmap) = driver.netmap()
        {
            netmaps.send_replace(Some(netmap));
        }
    }
}
