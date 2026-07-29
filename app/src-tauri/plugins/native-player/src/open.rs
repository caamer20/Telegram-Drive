use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    NativePlayerRequest, NativePlayerResult, NativePlayerSource, ResolvedStreamSource, Result,
};

pub(crate) struct OpenGuard<'a> {
    flag: &'a AtomicBool,
}

impl<'a> OpenGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Result<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| crate::Error::AlreadyOpen)?;
        Ok(Self { flag })
    }
}

impl Drop for OpenGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

pub(crate) async fn run_guarded_open<Resolve, Invoke, InvokeFuture>(
    flag: &AtomicBool,
    source: NativePlayerSource,
    resolve: Resolve,
    invoke: Invoke,
) -> Result<NativePlayerResult>
where
    Resolve: FnOnce(&NativePlayerSource) -> Result<ResolvedStreamSource>,
    Invoke: FnOnce(NativePlayerRequest) -> InvokeFuture,
    InvokeFuture: Future<Output = Result<NativePlayerResult>>,
{
    source.validate()?;
    let _guard = OpenGuard::acquire(flag)?;
    let resolved = resolve(&source)?;
    resolved.validate_loopback()?;
    invoke(NativePlayerRequest::new(source, resolved)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativePlayerError, NativePlayerErrorCategory, NativePlayerExitReason};
    use futures::{executor::block_on, poll};
    use std::future::pending;
    use std::task::Poll;

    fn source() -> NativePlayerSource {
        NativePlayerSource {
            folder_id: None,
            message_id: 7,
            title: "Movie".into(),
            file_name: Some("movie.mkv".into()),
            mime_type: Some("video/x-matroska".into()),
            start_position_ms: Some(0),
            autoplay: Some(true),
        }
    }

    fn resolved() -> ResolvedStreamSource {
        ResolvedStreamSource::direct("http://127.0.0.1:49152".into(), "secret".into())
    }

    fn result() -> NativePlayerResult {
        NativePlayerResult {
            position_ms: 1,
            duration_ms: 2,
            completed: false,
            exit_reason: NativePlayerExitReason::Back,
            error: Some(NativePlayerError {
                category: NativePlayerErrorCategory::Network,
                code: "TEST".into(),
                message: "Safe test error".into(),
            }),
            error_presented: false,
        }
    }

    #[test]
    fn success_resets_open_flag() {
        let flag = AtomicBool::new(false);
        let actual = block_on(run_guarded_open(
            &flag,
            source(),
            |_| Ok(resolved()),
            |_| async { Ok(result()) },
        ))
        .unwrap();
        assert_eq!(actual.position_ms, 1);
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn resolver_failure_resets_open_flag() {
        let flag = AtomicBool::new(false);
        let error = block_on(run_guarded_open(
            &flag,
            source(),
            |_| Err(crate::Error::StreamServer("resolver failed".into())),
            |_| async { Ok(result()) },
        ))
        .unwrap_err();
        assert!(matches!(error, crate::Error::StreamServer(_)));
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn invalid_source_does_not_lock_player() {
        let flag = AtomicBool::new(false);
        let mut invalid = source();
        invalid.message_id = 0;
        let error = block_on(run_guarded_open(
            &flag,
            invalid,
            |_| Ok(resolved()),
            |_| async { Ok(result()) },
        ))
        .unwrap_err();
        assert!(matches!(error, crate::Error::InvalidInput(_)));
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn native_invocation_failure_resets_open_flag() {
        let flag = AtomicBool::new(false);
        let error = block_on(run_guarded_open(
            &flag,
            source(),
            |_| Ok(resolved()),
            |_| async {
                Err(crate::Error::StreamServer(
                    "native invocation failed".into(),
                ))
            },
        ))
        .unwrap_err();
        assert!(matches!(error, crate::Error::StreamServer(_)));
        assert!(!flag.load(Ordering::Acquire));
    }

    #[test]
    fn concurrent_open_is_rejected_and_dropped_future_unlocks() {
        block_on(async {
            let flag = AtomicBool::new(false);
            let mut first = Box::pin(run_guarded_open(
                &flag,
                source(),
                |_| Ok(resolved()),
                |_| async { pending::<Result<NativePlayerResult>>().await },
            ));
            assert!(matches!(poll!(&mut first), Poll::Pending));
            assert!(flag.load(Ordering::Acquire));

            let duplicate = run_guarded_open(
                &flag,
                source(),
                |_| Ok(resolved()),
                |_| async { Ok(result()) },
            )
            .await
            .unwrap_err();
            assert!(matches!(duplicate, crate::Error::AlreadyOpen));

            drop(first);
            assert!(!flag.load(Ordering::Acquire));
            assert!(run_guarded_open(
                &flag,
                source(),
                |_| Ok(resolved()),
                |_| async { Ok(result()) },
            )
            .await
            .is_ok());
        });
    }
}
