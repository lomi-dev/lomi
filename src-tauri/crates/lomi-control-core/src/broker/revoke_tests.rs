use super::*;
use crate::terminal::TerminalControl;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_joins_detached_workers_and_survives_a_cancelled_close_caller() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("control");
    let broker = Broker::start(&directory).unwrap();
    broker.register_ui().unwrap();
    let weak = Arc::downgrade(&broker);
    let (started, ready) = oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let worker = broker.clone();
    drop(broker.spawn_worker(move || {
        started.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(worker.lock_state().is_err());
        // Even completion may schedule deferred settlement after shutdown began.
        worker.schedule_cleanup();
    }));
    ready.await.unwrap();
    let (started, ready) = oneshot::channel();
    let observer = broker.clone();
    broker.spawn_background(async move {
        let _observer = observer;
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    ready.await.unwrap();
    let closing = broker.clone();
    let first_close = tokio::spawn(async move { closing.shutdown().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !*broker.stop.borrow() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!first_close.is_finished());
    first_close.abort();
    let _ = first_close.await;
    let closing = broker.clone();
    let mut next_close = tokio::spawn(async move { closing.shutdown().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut next_close)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), next_close)
        .await
        .unwrap()
        .unwrap();
    assert!(broker
        .spawn_worker(|| panic!("closed broker admitted work"))
        .await
        .is_err());
    broker.shutdown().await;
    drop(broker);
    assert!(
        weak.upgrade().is_none(),
        "A worker retained the receipt owner"
    );
    let restarted = Broker::start(&directory).unwrap();
    restarted.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_invalidates_native_authority_without_waiting_for_policy_storage_or_observer() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    broker.register_ui().unwrap();
    let mut monitor = TerminalControl::new("owner".into(), "generation".into()).unwrap();
    monitor.bind_authorization(
        broker.authorization.clone(),
        broker.authorization.load(Ordering::SeqCst),
    );
    let lease = monitor.lease().unwrap().to_string();
    monitor.observe(b"\x1b]133;A\x07");
    let monitor = Arc::new(Mutex::new(monitor));
    {
        let observer = monitor.lock().unwrap();
        let policy = broker.state.lock().unwrap();
        let storage = broker.store.lock().unwrap();
        let before = Instant::now();
        broker.revoke();
        assert!(before.elapsed() < Duration::from_millis(100));
        assert!(!observer.authorized());
        assert_eq!(observer.lease(), None);
        assert!(broker.try_state().is_err());
        assert!(broker.cleanup_running.load(Ordering::SeqCst));
        drop(observer);
        drop(policy);
        drop(storage);
    }
    broker.shutdown().await;
    let mut observer = monitor.lock().unwrap();
    assert_eq!(
        observer
            .prepare_run(&lease, "old-request", "echo forbidden", false)
            .unwrap_err(),
        ErrorCode::ControlRevoked
    );
    broker.register_ui().unwrap();
    assert!(
        !observer.authorized(),
        "A new UI epoch never restores an old native token"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn disconnect_revokes_the_native_token_while_receipt_storage_is_blocked() {
    use crate::client::Client;
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let epoch = broker.register_ui().unwrap();
    broker
        .publish(Projection {
            ui_epoch: epoch,
            revision: "1".into(),
            workspaces: vec![Workspace {
                id: "workspace".into(),
                project_id: "project".into(),
                name: "fixture".into(),
                project_name: "fixture".into(),
                active_panel_id: None,
                project_path: root.path().canonicalize().unwrap().to_string_lossy().into(),
            }],
            ..Projection::default()
        })
        .unwrap();
    let endpoint = broker.endpoint.clone();
    let (pending, request) = tokio::sync::oneshot::channel();
    let connecting = tokio::spawn(async move {
        Client::connect(&endpoint, "blocked-storage", |id| {
            pending.send(id).unwrap();
        })
        .await
        .unwrap()
    });
    let pairing = request.await.unwrap();
    broker
        .approve_scopes(
            &pairing,
            &["workspace".into()],
            &["workspace.read".into(), "workspace.write".into()],
        )
        .unwrap();
    let client = connecting.await.unwrap();
    let Reply::Ok {
        data: Data::Connected { retry_epoch, .. },
        ..
    } = client
        .call(Request::Connect(ConnectInput {
            workspace_id: "workspace".into(),
        }))
        .await
        .unwrap()
    else {
        panic!("Expected connection");
    };
    let alive = broker.state.lock().unwrap().sessions[&pairing]
        .alive
        .clone();
    let mut monitor = TerminalControl::new(pairing, "test-generation".into()).unwrap();
    monitor.bind_authorization(
        broker.authorization.clone(),
        broker.authorization.load(Ordering::SeqCst),
    );
    monitor.bind_connection(alive.clone());
    assert!(monitor.lease().is_some());
    let dispatches = Arc::new(AtomicU64::new(0));
    let count = dispatches.clone();
    broker
        .set_ui_dispatch(Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }))
        .unwrap();
    let blocked_broker = broker.clone();
    let (locked, ready) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let blocker = std::thread::spawn(move || {
        let _storage = blocked_broker.store.lock().unwrap();
        locked.send(()).unwrap();
        let _ = held.recv_timeout(Duration::from_secs(5));
    });
    ready.await.unwrap();
    let call = tokio::spawn(async move {
        client
            .call(Request::RenameWorkspace(
                WorkspaceRenameInput {
                    workspace_id: "workspace".into(),
                    name: "must not apply".into(),
                    expected_revision: "1".into(),
                    retry_epoch,
                    request_key: "cancelled-client".into(),
                }
                .into(),
            ))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while broker.state.try_lock().is_ok() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let disconnected_at = Instant::now();
    call.abort();
    let _ = call.await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while alive.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("Disconnect must not wait for SQLite or the policy lock");
    assert!(
        disconnected_at.elapsed() < Duration::from_secs(2),
        "EOF cleanup blocked the only executor worker"
    );
    assert!(!monitor.authorized());
    assert_eq!(monitor.lease(), None);
    release.send(()).unwrap();
    blocker.join().unwrap();
    broker.shutdown().await;
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_older_registration_blocked_on_policy_cannot_replace_a_newer_ui() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let first = broker.register_ui().unwrap();
    let old;
    let new;
    {
        let policy = broker.state.lock().unwrap();
        let before = broker.authorization.load(Ordering::SeqCst);
        let first_broker = broker.clone();
        old = std::thread::spawn(move || first_broker.register_ui());
        let deadline = Instant::now() + Duration::from_secs(2);
        while broker.authorization.load(Ordering::SeqCst) == before {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let before = broker.authorization.load(Ordering::SeqCst);
        let second_broker = broker.clone();
        new = std::thread::spawn(move || second_broker.register_ui());
        while broker.authorization.load(Ordering::SeqCst) == before {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(policy);
    }
    assert!(old.join().unwrap().is_err());
    let current = new.join().unwrap().unwrap();
    assert_ne!(first, current);
    assert_eq!(broker.state.lock().unwrap().projection.ui_epoch, current);
    broker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_preparation_waits_for_detached_native_owners_and_can_resume() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let (started, ready) = oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let caller = broker.clone();
    let request = tokio::spawn(async move {
        caller
            .native_worker(move |_| {
                started.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(5)).unwrap();
            })
            .await
    });
    ready.await.unwrap();
    request.abort();
    let _ = request.await;
    broker.revoke();
    let closing = broker.clone();
    let mut preparing = tokio::spawn(async move { closing.pause_native_workers().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut preparing)
            .await
            .is_err()
    );
    assert_eq!(
        broker
            .native_worker(|_| panic!("close admitted native work"))
            .await,
        Err(ErrorCode::ControlRevoked)
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), preparing)
        .await
        .unwrap()
        .unwrap();
    broker.resume_native_workers();
    assert_eq!(broker.native_worker(|_| 42).await.unwrap(), 42);
    broker.shutdown().await;
    assert_eq!(
        broker.native_worker(|_| 43).await,
        Err(ErrorCode::ControlRevoked)
    );
}
