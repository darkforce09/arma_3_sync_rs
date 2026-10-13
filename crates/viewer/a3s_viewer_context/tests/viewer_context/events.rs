//! Coalescing of progress events, the repaint hook, and ordering across threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a3s_foundation::error::ErrorKind;
use a3s_sync::{EventError, Phase, SyncEvent, TransferProgress};
use a3s_viewer_context::{AppEvent, RepaintHook, TaskEvent, TaskKey, TaskKind, TaskOutcome};
use tokio::sync::oneshot;

use crate::support::{EVENT_TIMEOUT, Fixture};

fn counter() -> (Arc<AtomicUsize>, RepaintHook) {
    let count = Arc::new(AtomicUsize::new(0));
    let hook_count = Arc::clone(&count);
    let hook = RepaintHook::from_fn(move || {
        hook_count.fetch_add(1, Ordering::SeqCst);
    });
    (count, hook)
}

#[test]
fn progress_is_coalesced_into_the_snapshot() {
    let fixture = Fixture::new(Vec::new());
    let (repaints, hook) = counter();
    fixture.ctx.set_repaint_hook(hook);

    let (emitted, emitted_receiver) = std::sync::mpsc::channel::<()>();
    let (release, released) = oneshot::channel::<()>();
    let id = fixture
        .ctx
        .cx()
        .spawn_task(
            TaskKey::new(TaskKind::Download),
            false,
            move |task| async move {
                let events = task.sync_events();
                events.emit(SyncEvent::Phase(Phase::Downloading));
                for transferred in 0..1000 {
                    events.emit(SyncEvent::Transfer(TransferProgress {
                        transferred,
                        total: 1000,
                        ..TransferProgress::default()
                    }));
                }
                events.emit(SyncEvent::Speed {
                    bytes_per_second: 5,
                });
                events.emit(SyncEvent::FileDone {
                    relative_path: "a".to_owned(),
                });
                emitted.send(()).ok();
                released.await.ok();
                Ok(())
            },
        )
        .expect("spawn");

    emitted_receiver
        .recv_timeout(EVENT_TIMEOUT)
        .expect("task emitted");
    let snapshot = fixture.ctx.task_snapshot(id).expect("running task");
    assert_eq!(snapshot.transfer.expect("transfer").transferred, 999);
    assert_eq!(snapshot.speed, Some(5));
    assert_eq!(snapshot.phase, Some(Phase::Downloading));
    // Started, Phase and FileDone were queued, and 1 001 snapshot updates made; each repaints.
    assert!(repaints.load(Ordering::SeqCst) >= 1004);
    release.send(()).expect("task waiting");

    let events = fixture.until(|event| matches!(event, AppEvent::Task(TaskEvent::Finished { .. })));
    assert_eq!(events.len(), 4, "{events:#?}");
    assert!(matches!(&events[0], AppEvent::Task(TaskEvent::Started(info)) if info.id == id));
    assert!(matches!(
        &events[1],
        AppEvent::Task(TaskEvent::Sync { id: event_id, event: SyncEvent::Phase(Phase::Downloading) })
            if *event_id == id
    ));
    assert!(matches!(
        &events[2],
        AppEvent::Task(TaskEvent::Sync { event: SyncEvent::FileDone { relative_path }, .. })
            if relative_path == "a"
    ));
    assert!(matches!(
        &events[3],
        AppEvent::Task(TaskEvent::Finished {
            outcome: TaskOutcome::Completed,
            ..
        })
    ));
    assert!(fixture.ctx.task_snapshot(id).is_none());
}

#[test]
fn set_repaint_hook_replaces_the_hook() {
    let fixture = Fixture::new(Vec::new());
    let (first, hook) = counter();
    fixture.ctx.set_repaint_hook(hook);
    fixture.ctx.cx().emit(error_event("first", 0));
    assert_eq!(first.load(Ordering::SeqCst), 1);

    let (second, hook) = counter();
    fixture.ctx.set_repaint_hook(hook);
    fixture.ctx.cx().emit(error_event("second", 0));
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.ctx.poll_events().len(), 2);
    assert!(fixture.ctx.poll_events().is_empty());
}

fn error_event(context: &str, index: usize) -> AppEvent {
    AppEvent::Error {
        context: context.to_owned(),
        error: EventError::new(ErrorKind::Internal, index.to_string()),
    }
}

#[test]
fn events_from_many_threads_keep_their_order_per_thread() {
    let fixture = Fixture::new(Vec::new());
    let threads: Vec<_> = (0..4)
        .map(|thread| {
            let cx = fixture.ctx.cx().clone();
            std::thread::Builder::new()
                .name(format!("emitter-{thread}"))
                .spawn(move || {
                    for index in 0..100 {
                        cx.emit(error_event(&thread.to_string(), index));
                    }
                })
                .expect("spawn thread")
        })
        .collect();
    for thread in threads {
        thread.join().expect("emitter");
    }

    let mut next_index = [0_usize; 4];
    for _ in 0..400 {
        let Some(AppEvent::Error { context, error }) =
            fixture.ctx.next_event_timeout(EVENT_TIMEOUT)
        else {
            panic!("expected an error event");
        };
        let thread: usize = context.parse().expect("thread number");
        assert_eq!(error.message, next_index[thread].to_string());
        next_index[thread] += 1;
    }
    assert_eq!(next_index, [100; 4]);
}
