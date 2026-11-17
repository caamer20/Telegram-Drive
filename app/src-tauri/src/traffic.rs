//! Aggregate content pacing and local civil-time bandwidth windows.
use jiff::civil::DateTime;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, ReadBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    /// Monday = 0. An overnight window belongs to its starting day.
    pub days: Vec<u8>,
    pub start_minute: u16,
    pub end_minute: u16,
    pub up_kbs: u32,
    pub down_kbs: u32,
    pub pause: bool,
}
impl Window {
    pub fn validate(&self) -> Result<(), String> {
        let days: std::collections::HashSet<_> = self.days.iter().copied().collect();
        if self.days.is_empty()
            || days.len() != self.days.len()
            || self.days.iter().any(|day| *day > 6)
            || self.start_minute >= 1440
            || self.end_minute > 1440
            || self.start_minute == self.end_minute
            || self.up_kbs > 65536
            || self.down_kbs > 65536
        {
            return Err("Invalid bandwidth schedule window".into());
        }
        Ok(())
    }
    fn active(&self, now: DateTime) -> bool {
        let day = now.weekday().to_monday_zero_offset() as u8;
        let minute = now.hour() as u16 * 60 + now.minute() as u16;
        if self.start_minute < self.end_minute {
            self.days.contains(&day) && minute >= self.start_minute && minute < self.end_minute
        } else {
            (self.days.contains(&day) && minute >= self.start_minute)
                || (self.days.contains(&((day + 6) % 7)) && minute < self.end_minute)
        }
    }
}
#[derive(Clone, Copy)]
pub enum Direction {
    Upload,
    Download,
}
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Policy {
    rate: u64,
    pause: bool,
    windows: u32,
    revision: u64,
}
struct Queued {
    id: u64,
    duration: Duration,
}
struct Lane {
    policy: Policy,
    cursor: Instant,
    queue: std::collections::VecDeque<Queued>,
    next_id: u64,
}
struct Ticket<'a> {
    pacer: &'a Pacer,
    lane: usize,
    id: u64,
    policy: Policy,
    granted: bool,
}
impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        if self.granted {
            return;
        }
        if let Ok(mut lanes) = self.pacer.lanes.lock() {
            let lane = &mut lanes[self.lane];
            if lane.policy == self.policy {
                if let Some(position) = lane.queue.iter().position(|queued| queued.id == self.id) {
                    lane.queue.remove(position);
                    if position == 0 {
                        lane.cursor = Instant::now();
                    }
                }
            }
        }
        self.pacer.changed.notify_waiters();
    }
}
#[cfg(feature = "native-e2e")]
static ACTIVE_WAITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "native-e2e")]
struct WaitObservation;
#[cfg(feature = "native-e2e")]
impl WaitObservation {
    fn begin() -> Self {
        ACTIVE_WAITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}
#[cfg(feature = "native-e2e")]
impl Drop for WaitObservation {
    fn drop(&mut self) {
        ACTIVE_WAITS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub struct Pacer {
    lanes: Mutex<[Lane; 2]>,
    changed: tokio::sync::Notify,
    revision: std::sync::atomic::AtomicU64,
    #[cfg(feature = "native-e2e")]
    clock: std::sync::RwLock<Option<DateTime>>,
}
impl Default for Pacer {
    fn default() -> Self {
        Self {
            lanes: Mutex::new(std::array::from_fn(|_| Lane {
                policy: Policy::default(),
                cursor: Instant::now(),
                queue: Default::default(),
                next_id: 0,
            })),
            changed: tokio::sync::Notify::new(),
            revision: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "native-e2e")]
            clock: std::sync::RwLock::new(None),
        }
    }
}
impl Pacer {
    #[cfg(feature = "native-e2e")]
    pub fn pending(&self) -> usize {
        ACTIVE_WAITS.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn changed(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.changed.notify_waiters();
    }
    #[cfg(feature = "native-e2e")]
    pub fn set_clock(&self, now: DateTime) {
        *self.clock.write().unwrap_or_else(|e| e.into_inner()) = Some(now);
        self.changed();
    }
    fn now(&self) -> DateTime {
        #[cfg(feature = "native-e2e")]
        if let Some(now) = *self.clock.read().unwrap_or_else(|e| e.into_inner()) {
            return now;
        }
        crate::app_time::local_datetime()
    }
    fn policy(&self, config: &crate::vpn_optimizer::NetworkConfig, direction: Direction) -> Policy {
        let vpn = config.vpn.read().unwrap_or_else(|e| e.into_inner());
        let mut policy = Policy {
            rate: match direction {
                Direction::Download => u64::from(vpn.bandwidth_limit_down_kbs) * 1024,
                Direction::Upload if vpn.bandwidth_schedule => {
                    u64::from(vpn.bandwidth_limit_up_kbs) * 1024
                }
                Direction::Upload => 0,
            },
            revision: self.revision.load(std::sync::atomic::Ordering::SeqCst),
            ..Policy::default()
        };
        if vpn.bandwidth_schedule {
            let now = self.now();
            for (index, window) in vpn.bandwidth_windows.iter().take(32).enumerate() {
                if !window.active(now) {
                    continue;
                }
                policy.windows |= 1 << index;
                policy.pause |= window.pause;
                let rate = u64::from(match direction {
                    Direction::Upload => window.up_kbs,
                    Direction::Download => window.down_kbs,
                }) * 1024;
                if rate > 0 && (policy.rate == 0 || rate < policy.rate) {
                    policy.rate = rate;
                }
            }
        }
        policy
    }
    pub async fn wait(
        &self,
        config: &crate::vpn_optimizer::NetworkConfig,
        direction: Direction,
        bytes: usize,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        if bytes == 0 {
            return check();
        }
        #[cfg(feature = "native-e2e")]
        let _observation = WaitObservation::begin();
        let lane_index = match direction {
            Direction::Upload => 0,
            Direction::Download => 1,
        };
        let mut ticket: Option<Ticket<'_>> = None;
        loop {
            check()?;
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let (policy, id, granted, delay) = {
                let mut lanes = self
                    .lanes
                    .lock()
                    .map_err(|_| "Bandwidth pacing unavailable")?;
                // Read the live policy inside the lane lock: a waiter carrying
                // an earlier snapshot cannot overwrite a newer queue.
                let policy = self.policy(config, direction);
                let lane = &mut lanes[lane_index];
                if lane.policy != policy {
                    lane.policy = policy;
                    lane.cursor = Instant::now();
                    lane.queue.clear();
                }
                if policy.pause || policy.rate == 0 {
                    (policy, None, false, Duration::from_millis(250))
                } else {
                    let id = match ticket.as_ref().filter(|ticket| {
                        ticket.policy == policy
                            && lane.queue.iter().any(|queued| queued.id == ticket.id)
                    }) {
                        Some(ticket) => ticket.id,
                        None => {
                            if lane.queue.len() >= 512 {
                                return Err("Too many paced transfers".into());
                            }
                            lane.next_id = lane
                                .next_id
                                .checked_add(1)
                                .ok_or("Bandwidth pacing identifiers exhausted")?;
                            if lane.queue.is_empty() {
                                lane.cursor = Instant::now();
                            }
                            let id = lane.next_id;
                            lane.queue.push_back(Queued {
                                id,
                                duration: Duration::from_secs_f64(
                                    bytes as f64 / policy.rate as f64,
                                ),
                            });
                            id
                        }
                    };
                    let mut deadline = lane.cursor;
                    for queued in &lane.queue {
                        deadline += queued.duration;
                        if queued.id == id {
                            break;
                        }
                    }
                    let head = lane.queue.front().is_some_and(|queued| queued.id == id);
                    let granted = head && Instant::now() >= deadline;
                    if granted {
                        lane.queue.pop_front();
                        lane.cursor = Instant::now();
                    }
                    let delay = if !head && Instant::now() >= deadline {
                        Duration::from_millis(25)
                    } else {
                        deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(250))
                    };
                    (policy, Some(id), granted, delay)
                }
            };
            // Dropping a ticket may acquire the lane lock, so all replacements
            // and successful-grant cleanup happen after releasing it.
            if let Some(id) = id {
                if !ticket
                    .as_ref()
                    .is_some_and(|ticket| ticket.policy == policy && ticket.id == id)
                {
                    ticket = Some(Ticket {
                        pacer: self,
                        lane: lane_index,
                        id,
                        policy,
                        granted,
                    });
                }
                if granted {
                    if let Some(ticket) = ticket.as_mut() {
                        ticket.granted = true;
                    }
                    self.changed.notify_waiters();
                    return check();
                }
            } else {
                ticket = None;
            }
            if !policy.pause && policy.rate == 0 {
                return check();
            }
            tokio::select! {_=&mut changed=>{},_=tokio::time::sleep(delay)=>{}}
        }
    }
}
#[derive(Clone)]
pub struct Traffic {
    pub network: Arc<crate::vpn_optimizer::NetworkConfig>,
    pub account: crate::workspace::AccountGuard,
    pub direction: Direction,
}
impl Traffic {
    pub async fn wait(&self, bytes: usize) -> Result<(), String> {
        self.network
            .pacer
            .wait(&self.network, self.direction, bytes, || {
                self.account.validate()
            })
            .await
    }
}
/// Bounded content adapter used by the SDK's ordinary upload path. Completed
/// resumable prefixes are verified locally, so only the part sink is paced.
type PermitFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

pub struct Reader<R> {
    inner: R,
    traffic: Traffic,
    bytes: Vec<u8>,
    offset: usize,
    pending: Option<PermitFuture>,
    failure: Option<String>,
}
impl<R> Reader<R> {
    pub fn new(inner: R, traffic: Traffic) -> Self {
        Self {
            inner,
            traffic,
            bytes: Vec::new(),
            offset: 0,
            pending: None,
            failure: None,
        }
    }
}
impl<R: AsyncRead + Unpin> AsyncRead for Reader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if let Some(error) = &self.failure {
            return Poll::Ready(Err(std::io::Error::other(error.clone())));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if let Some(pending) = self.pending.as_mut() {
            match pending.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => {
                    self.pending = None;
                    self.failure = Some(error.clone());
                    return Poll::Ready(Err(std::io::Error::other(error)));
                }
                Poll::Ready(Ok(())) => self.pending = None,
            }
        }
        if self.offset < self.bytes.len() {
            if let Err(error) = self.traffic.account.validate() {
                self.failure = Some(error.clone());
                return Poll::Ready(Err(std::io::Error::other(error)));
            }
            let count = output.remaining().min(self.bytes.len() - self.offset);
            output.put_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            return Poll::Ready(Ok(()));
        }
        let mut bytes = vec![0; output.remaining().min(65536)];
        let mut input = ReadBuf::new(&mut bytes);
        match Pin::new(&mut self.inner).poll_read(cx, &mut input) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {
                let count = input.filled().len();
                if count == 0 {
                    return Poll::Ready(Ok(()));
                }
                bytes.truncate(count);
                self.bytes = bytes;
                self.offset = 0;
                let traffic = self.traffic.clone();
                self.pending = Some(Box::pin(async move { traffic.wait(count).await }));
                // Poll again through the same state; ready bytes are never exposed
                // before their aggregate allowance has elapsed.
                self.poll_read(cx, output)
            }
        }
    }
}
