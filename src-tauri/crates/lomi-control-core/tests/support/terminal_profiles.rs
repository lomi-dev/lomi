use super::*;

#[tokio::test]
async fn selected_shell_is_pinned_per_session_and_revalidated_at_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let broker = Broker::start(&root.path().join("control")).unwrap();
    let mut p = projection(&broker, root.path());
    let zsh = TerminalProfile {
        id: "local:zsh".into(),
        revision: "zsh-pinned".into(),
    };
    let bash = TerminalProfile {
        id: "local:bash".into(),
        revision: "bash-pinned".into(),
    };
    p.terminal_profile = Some(zsh.clone());
    p.terminal_profiles = vec![zsh.clone(), bash.clone()];
    broker.publish(p.clone()).unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    broker
        .set_ui_dispatch(Arc::new(move |command| {
            tx.send(command).map_err(std::io::Error::other)
        }))
        .unwrap();
    let scopes = [
        "workspace.read",
        "panel.create",
        "terminal.execute",
        "terminal.read",
    ];
    for selected in [&bash, &zsh] {
        let client =
            approved_profile(&broker, &["a"], &scopes, &[], &[], &[], Some(&selected.id)).await;
        let Reply::Ok {
            data: Data::Connected { retry_epoch, .. },
            ..
        } = client
            .call(Request::Connect(ConnectInput {
                workspace_id: "a".into(),
            }))
            .await
            .unwrap()
        else {
            panic!()
        };
        let mut input = TerminalCreateInput {
            workspace_id: "a".into(),
            cwd_relative: ".".into(),
            profile_id: None,
            title: "Selected shell".into(),
            expected_revision: p.revision.clone(),
            retry_epoch,
            request_key: "selected-shell".into(),
        };
        input.profile_id = Some(if selected.id == bash.id {
            zsh.id.clone()
        } else {
            bash.id.clone()
        });
        assert!(matches!(
            client
                .call(Request::CreateTerminal(input.clone()))
                .await
                .unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        assert!(commands.try_recv().is_err());
        input.profile_id = None;
        assert!(matches!(
            client
                .call(Request::CreateTerminal(input.clone()))
                .await
                .unwrap(),
            Reply::Ok { .. }
        ));
        let command = commands.recv().await.unwrap();
        let UiAction::CreateTerminal {
            ref profile_id,
            ref terminal_session_id,
            ref cwd,
            ..
        } = command.action
        else {
            panic!()
        };
        assert_eq!(profile_id, &selected.id);
        broker
            .claim_ui(&p.ui_epoch, &command.operation_id, &command.nonce)
            .unwrap();
        p.terminal_profiles
            .iter_mut()
            .find(|v| v.id == selected.id)
            .unwrap()
            .revision = "changed".into();
        // The legacy default cannot override a changed qualified profile.
        p.revision = (p.revision.parse::<u64>().unwrap() + 1).to_string();
        broker.publish(p.clone()).unwrap();
        input.expected_revision = p.revision.clone();
        assert!(broker
            .start_terminal(
                &command.operation_id,
                &command.nonce,
                terminal_session_id,
                selected,
                cwd,
                |_| -> Result<(), String> { panic!("Stale approval spawned a shell") }
            )
            .is_err());
        input.request_key = "changed-profile".into();
        assert!(matches!(
            client.call(Request::CreateTerminal(input)).await.unwrap(),
            Reply::Error {
                code: ErrorCode::ScopeDenied,
                ..
            }
        ));
        p.terminal_profiles = vec![zsh.clone(), bash.clone()];
        p.revision = (p.revision.parse::<u64>().unwrap() + 1).to_string();
        broker.publish(p.clone()).unwrap();
    }
    broker.shutdown().await;
}
