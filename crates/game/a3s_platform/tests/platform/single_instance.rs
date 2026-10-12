//! The single-instance lock: owner, second instance, release, squatters and bad clients.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use a3s_foundation::paths::Os;
use a3s_platform::single_instance::{
    Acquire, AcquireOptions, InstanceId, InstanceMessage, acquire,
};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};

/// A lock id no other test or process uses.
fn unique_id() -> InstanceId {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    InstanceId::from_raw(format!(
        "a3s-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Options whose socket folder (used on Unixes without abstract sockets) is `dir`.
fn options(dir: &Path) -> AcquireOptions {
    AcquireOptions {
        socket_dir: Some(dir.to_path_buf()),
        ..AcquireOptions::default()
    }
}

/// The socket name `acquire` uses for `id`.
fn socket_name(id: &InstanceId, dir: &Path) -> Name<'static> {
    if GenericNamespaced::is_supported() && cfg!(any(windows, target_os = "linux")) {
        id.as_str()
            .to_owned()
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    } else {
        dir.join(format!("{}.sock", id.as_str()))
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    }
}

fn channel_callback() -> (
    impl Fn(InstanceMessage) + Send + Sync + 'static,
    mpsc::Receiver<InstanceMessage>,
) {
    let (sender, receiver) = mpsc::channel();
    let sender = parking_lot::Mutex::new(sender);
    let callback = move |message| {
        let _ignored = sender.lock().send(message);
    };
    (callback, receiver)
}

#[test]
fn second_instance_activates_the_first_until_it_exits() {
    let temp = tempfile::tempdir().unwrap();
    let id = unique_id();
    let (callback, messages) = channel_callback();

    let Acquire::Primary(guard) = acquire(&id, &options(temp.path()), callback) else {
        panic!("the first instance must own the lock");
    };
    assert!(matches!(
        acquire(&id, &options(temp.path()), |_| {}),
        Acquire::Secondary
    ));
    assert_eq!(
        messages.recv_timeout(Duration::from_secs(2)),
        Ok(InstanceMessage::Activate)
    );

    guard.shutdown();
    let Acquire::Primary(third) = acquire(&id, &options(temp.path()), |_| {}) else {
        panic!("a released lock can be taken again");
    };
    drop(third);
}

#[test]
fn a_silent_squatter_cannot_block_startup() {
    let temp = tempfile::tempdir().unwrap();
    let id = unique_id();
    let squatter = ListenerOptions::new()
        .name(socket_name(&id, temp.path()))
        .create_sync()
        .unwrap();
    // Accept and hold every connection without answering.
    thread::Builder::new()
        .name("squatter".to_owned())
        .spawn(move || {
            let mut held = Vec::new();
            for stream in squatter.incoming().flatten() {
                held.push(stream);
            }
        })
        .unwrap();

    let started = Instant::now();
    let options = AcquireOptions {
        attempts: 2,
        reply_timeout: Duration::from_millis(200),
        ..options(temp.path())
    };
    assert!(matches!(
        acquire(&id, &options, |_| {}),
        Acquire::Unlocked { .. }
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn unknown_requests_get_no_answer_and_no_callback() {
    let temp = tempfile::tempdir().unwrap();
    let id = unique_id();
    let (callback, messages) = channel_callback();
    let Acquire::Primary(_guard) = acquire(&id, &options(temp.path()), callback) else {
        panic!("the first instance must own the lock");
    };

    let mut client = LocalSocketStream::connect(socket_name(&id, temp.path())).unwrap();
    client.write_all(b"garbage\n").unwrap();
    client.flush().unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).unwrap();
    assert!(reply.is_empty());
    assert!(messages.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn instance_ids_are_per_config_folder() {
    // FNV-1a-64 of "a" is af63dc4c8601ec8c (the published test vector).
    assert_eq!(
        InstanceId::for_config_dir(Path::new("a"), Os::Linux).as_str(),
        "arma3sync-rs-af63dc4c8601ec8c"
    );
    let alice = InstanceId::for_config_dir(Path::new("/home/a/.config/arma3sync-rs"), Os::Linux);
    assert_eq!(
        alice,
        InstanceId::for_config_dir(Path::new("/home/a/.config/arma3sync-rs"), Os::Linux)
    );
    assert_ne!(
        alice,
        InstanceId::for_config_dir(Path::new("/home/b/.config/arma3sync-rs"), Os::Linux)
    );
    assert_ne!(
        InstanceId::for_config_dir(Path::new("/A"), Os::Linux),
        InstanceId::for_config_dir(Path::new("/a"), Os::Linux)
    );
    assert_eq!(
        InstanceId::for_config_dir(
            Path::new(r"C:\Users\Bob\AppData\Roaming\arma3sync-rs"),
            Os::Windows
        ),
        InstanceId::for_config_dir(
            Path::new(r"c:\users\bob\appdata\roaming\ARMA3SYNC-RS"),
            Os::Windows
        )
    );
}
