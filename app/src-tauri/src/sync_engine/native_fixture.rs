//! Controlled Telegram boundary for the opt-in native process journeys.
//! The production engine, watchers, local scanner, planner and SQLite remain real.
use super::{planner::FileTree, SyncPair};
use crate::workspace::AccountGuard;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Listener, Manager};
use tokio::sync::Notify;

pub(crate) struct NativeSyncFixture {
    pub root: PathBuf,
    owner: Mutex<Option<i64>>,
    remote: Mutex<HashMap<i64, FileTree>>,
    pub scans: AtomicUsize,
    pub probes: AtomicUsize,
    history: Mutex<Vec<Value>>,
    changed: Notify,
}

impl NativeSyncFixture {
    pub fn new(root: PathBuf) -> Self {
        let remote = std::fs::read(root.join("sync-fixture-remote.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            root,
            owner: Mutex::new(None),
            remote: Mutex::new(remote),
            scans: AtomicUsize::new(0),
            probes: AtomicUsize::new(0),
            history: Mutex::new(Vec::new()),
            changed: Notify::new(),
        }
    }

    pub fn set_owner(&self, owner: Option<i64>) {
        *self.owner.lock().unwrap() = owner;
    }

    pub fn probe(&self) -> Option<i64> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        *self.owner.lock().unwrap()
    }

    pub fn owner(&self) -> Option<i64> {
        *self.owner.lock().unwrap()
    }

    pub async fn scan(&self, pair: &SyncPair, account: &AccountGuard) -> Result<FileTree, String> {
        account.validate()?;
        self.scans.fetch_add(1, Ordering::SeqCst);
        match self.owner() {
            Some(owner) if owner == account.owner => Ok(self
                .remote
                .lock()
                .unwrap()
                .get(&pair.id)
                .cloned()
                .unwrap_or_default()),
            Some(_) => Err("ACCOUNT_CHANGED: Fixture Telegram account changed".into()),
            None => Err(
                "Telegram is offline; remote tree unavailable, so no reconciliation was attempted"
                    .into(),
            ),
        }
    }

    pub fn install_listener(app: &tauri::AppHandle) {
        let handle = app.clone();
        app.listen("sync-status-changed", move |event| {
            // SyncStatus is serialize-only in production; decode just the wire
            // fields needed to capture observable native status transitions.
            let Ok(value) = serde_json::from_str::<Value>(event.payload()) else {
                return;
            };
            let Some(fixture) = handle.try_state::<Arc<NativeSyncFixture>>() else {
                return;
            };
            let status = value;
            fixture.history.lock().unwrap().push(status.clone());
            // Wake readers only after the engine has published the new snapshot.
            if status["running"] == false {
                fixture.changed.notify_one();
            }
        });
    }

    pub async fn snapshot(
        &self,
        app: &tauri::AppHandle,
        expected: Option<&str>,
    ) -> Result<Value, String> {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                let status = app.state::<super::SyncEngine>().status.read().await.clone();
                let value = serde_json::to_value(&status).map_err(|error| error.to_string())?;
                let pairs = value["pairs"].as_array().cloned().unwrap_or_default();
                let matches = match expected {
                    Some("waiting") => {
                        !pairs.is_empty()
                            && pairs.iter().all(|pair| {
                                pair["phase"] == "paused" && pair["lastError"] == "Paused by you"
                                    || pair["waitReason"] == "telegram"
                                    || !pair["lastError"].is_null()
                            })
                    }
                    Some("ready") => {
                        !pairs.is_empty()
                            && pairs.iter().all(|pair| {
                                pair["phase"] == "ready"
                                    || pair["phase"] == "paused"
                                        && pair["lastError"] == "Paused by you"
                            })
                    }
                    Some("error") => pairs.iter().any(|pair| {
                        !pair["lastError"].is_null() && pair["lastError"] != "Paused by you"
                    }),
                    _ => true,
                };
                if matches && !status.running {
                    return Ok(json!({
                        "status":value,
                        "scans":self.scans.load(Ordering::SeqCst),
                        "probes":self.probes.load(Ordering::SeqCst),
                        "history":self.history.lock().unwrap().clone(),
                    }));
                }
                notified.await;
            }
        };
        tokio::time::timeout(Duration::from_secs(8), wait)
            .await
            .map_err(|_| format!("Sync did not publish the expected {expected:?} state"))?
    }
}
