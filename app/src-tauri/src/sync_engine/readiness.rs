//! Process-local Telegram readiness. Notifications are retained across engine
//! starts, and a late reply from a replaced connection cannot make it ready.
use std::time::Duration;
use tauri::Manager;
use tokio::sync::watch;

pub(super) const RETRY_MIN: Duration = Duration::from_secs(2);
pub(super) const RETRY_MAX: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct ProbeResult {
    pub error: Option<String>,
    pub retry_after: Option<Duration>,
}

/// Checks only Telegram/account readiness; never scans folders, mutates a
/// session, or starts another network runner. The engine cancels this on shutdown.
pub(super) async fn probe(app: &tauri::AppHandle) -> ProbeResult {
    let telegram = app.state::<crate::commands::TelegramState>();
    let (client, generation) = {
        let client = telegram.client.lock().await;
        (client.clone(), telegram.sync_readiness.generation())
    };
    #[cfg(feature = "native-e2e")]
    if let Some(fixture) =
        app.try_state::<std::sync::Arc<super::native_fixture::NativeSyncFixture>>()
    {
        if let Some(owner) = fixture.probe() {
            match crate::workspace::AccountGuard::open(&fixture.root, Some(&owner.to_string())) {
                Ok(_) => {
                    telegram.sync_readiness.mark_ready(generation, owner);
                }
                Err(error) => {
                    telegram.sync_readiness.mark_waiting(generation);
                    return ProbeResult {
                        error: Some(error),
                        ..ProbeResult::default()
                    };
                }
            }
        } else {
            telegram.sync_readiness.mark_waiting(generation);
        }
        return ProbeResult::default();
    }
    let Some(client) = client else {
        telegram.sync_readiness.mark_waiting(generation);
        return ProbeResult::default();
    };
    let account = match super::sync_data_dir(app)
        .and_then(|root| crate::workspace::AccountGuard::open(&root, None))
    {
        Ok(account) => account,
        Err(error) => {
            telegram.sync_readiness.mark_waiting(generation);
            return ProbeResult {
                error: Some(error),
                ..ProbeResult::default()
            };
        }
    };
    let response = tokio::time::timeout(Duration::from_secs(5), client.get_me()).await;
    // An old probe must not publish either success or failure for its replacement.
    if telegram.sync_readiness.generation() != generation {
        return ProbeResult::default();
    }
    match response {
        Ok(Ok(user)) => {
            if user.bare_id() != account.owner {
                telegram.sync_readiness.mark_waiting(generation);
                return ProbeResult {
                    error: Some(
                        "ACCOUNT_CHANGED: Connected client belongs to another account".into(),
                    ),
                    ..ProbeResult::default()
                };
            }
            if let Err(error) = account.validate() {
                telegram.sync_readiness.mark_waiting(generation);
                return ProbeResult {
                    error: Some(error),
                    ..ProbeResult::default()
                };
            }
            telegram
                .sync_readiness
                .mark_ready(generation, account.owner);
            ProbeResult::default()
        }
        Err(_) => {
            telegram.sync_readiness.mark_waiting(generation);
            ProbeResult::default()
        }
        Ok(Err(error)) => {
            use grammers_mtsender::InvocationError;
            telegram.sync_readiness.mark_waiting(generation);
            if let Some(seconds) = crate::commands::utils::flood_wait_seconds(&error.to_string()) {
                return ProbeResult {
                    error: None,
                    retry_after: Some(Duration::from_secs(seconds.max(1))),
                };
            }
            match error {
                InvocationError::Io(_)
                | InvocationError::Transport(_)
                | InvocationError::Dropped => ProbeResult::default(),
                InvocationError::Rpc(ref rpc) if rpc.code >= 500 => ProbeResult::default(),
                _ => ProbeResult {
                    error: Some(error.to_string()),
                    ..ProbeResult::default()
                },
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub generation: u64,
    pub owner: Option<i64>,
}

#[derive(Clone)]
pub struct SyncReadiness {
    sender: watch::Sender<Snapshot>,
}

impl Default for SyncReadiness {
    fn default() -> Self {
        let (sender, _) = watch::channel(Snapshot::default());
        Self { sender }
    }
}

impl SyncReadiness {
    pub fn snapshot(&self) -> Snapshot {
        self.sender.borrow().clone()
    }

    pub fn generation(&self) -> u64 {
        self.sender.borrow().generation
    }

    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.sender.subscribe()
    }

    pub fn invalidate(&self) -> u64 {
        let mut generation = 0;
        self.sender.send_modify(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.owner = None;
            generation = state.generation;
        });
        generation
    }

    pub fn invalidate_if_generation(&self, expected: u64) -> bool {
        self.sender.send_if_modified(|state| {
            if state.generation != expected {
                return false;
            }
            state.generation = state.generation.wrapping_add(1);
            state.owner = None;
            true
        })
    }

    pub fn mark_ready(&self, generation: u64, owner: i64) -> bool {
        self.sender.send_if_modified(|state| {
            if state.generation != generation || state.owner == Some(owner) || owner <= 0 {
                return false;
            }
            state.owner = Some(owner);
            true
        })
    }

    pub fn mark_waiting(&self, generation: u64) -> bool {
        self.sender.send_if_modified(|state| {
            if state.generation != generation || state.owner.is_none() {
                return false;
            }
            state.owner = None;
            true
        })
    }
}
