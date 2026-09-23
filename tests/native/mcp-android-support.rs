//! Opt-in P0 fixture using the production manager and its owned native actor.
use super::{
    adb, auth,
    devices::{Action, Draft},
    input::{Event, Request, Router, TextAction},
    installer::Phase,
    manager::Android,
    storage::{Gpu, Hardware},
};
use std::{fs, path::PathBuf, sync::atomic::Ordering, time::Duration};

#[tokio::test]
#[ignore = "Creates and retains a stopped device in the explicitly accepted isolated MCP SDK"]
async fn prepare_and_verify_isolated_mcp_device() {
    let trial = PathBuf::from(std::env::var_os("LOMI_ANDROID_PROBE_DIRECTORY").unwrap())
        .canonicalize()
        .unwrap();
    assert!(trial
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("lomi-android-stage0-"));
    let read =
        |path| serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap();
    assert_eq!(read(trial.join("evidence/consent.json"))["accepted"], true);
    let installation = read(trial.join("evidence/native-managed-installation.json"));
    let root = PathBuf::from(installation["root"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(root.parent(), Some(trial.as_path()));
    let report_path = trial.join("evidence/mcp-device.json");
    let android = Android::default();
    let manager = android.get(root.clone()).unwrap();
    manager.test_adb_port.store(15047, Ordering::Release);
    assert!(
        std::net::TcpListener::bind(("127.0.0.1", 15047)).is_ok(),
        "Private fixture ADB port is already in use"
    );
    let id = if report_path.exists() {
        let previous = read(report_path.clone());
        assert_eq!(previous["root"], root.to_string_lossy().as_ref());
        previous["deviceId"].as_str().unwrap().to_owned()
    } else {
        let revision = manager
            .directory
            .lock()
            .unwrap()
            .devices()
            .unwrap()
            .revision;
        manager
            .installer
            .manage(
                &manager,
                Action::Create {
                    expected_revision: revision,
                    draft: Draft {
                        name: "MCP qualification".into(),
                        image: "system-images;android-36;default;arm64-v8a".into(),
                        profile: "small_phone".into(),
                        hardware: Hardware {
                            ram_mib: 2560,
                            cpu_count: 2,
                            data_gib: 6,
                            gpu: Gpu::Host,
                            quick_boot: false,
                        },
                    },
                },
            )
            .unwrap();
        manager.installer.settle(false).await.unwrap();
        let result = manager.installer.progress().unwrap();
        assert_eq!(result.phase, Phase::Succeeded, "{result:?}");
        let id = result.device_id.unwrap();
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(
                &serde_json::json!({"root":root,"deviceId":id,"state":"prepared"}),
            )
            .unwrap(),
        )
        .unwrap();
        id
    };
    let result: Result<serde_json::Value,String> = async {
        let started = manager.start(&id).await?;
        let generation = started.generation.ok_or("Missing native generation")?;
        let runtime = manager.runtime(&id,&generation)?;
        let apk = trial.join("development-tools/input-build/input-test.apk");
        let bytes = fs::read(&apk).map_err(|e|e.to_string())?;
        if bytes.len()>1024*1024 { return Err("Fixture APK exceeded its size limit".into()); }
        runtime.install_apk(generation.clone(),fs::File::open(&apk).map_err(|e|e.to_string())?).await?;
        let record = read(root.join("runtime").join(format!("{id}.json")));
        let guest = adb::Guest {
            server: adb::Server { port:15047 },console_port: record["consolePort"].as_u64().ok_or("Missing console port")? as u16,
            device_id:id.clone(),generation_key:record["generationKey"].as_str().ok_or("Missing guest generation")?.into(),
        };
        let open = guest.clone();
        tokio::task::spawn_blocking(move||open.native_input_fixture(true)).await.map_err(|e|e.to_string())??;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let lease = Router::submit(manager.clone(),id.clone(),generation.clone(),Request::Focus {view_id:auth::new_id()?}).await?.lease.ok_or("Missing focus lease")?;
        Router::submit(manager.clone(),id.clone(),generation.clone(),Request::Send {
            lease:lease.clone(),sequence:1,event:Event::Text {action:TextAction::Commit,text:"Zażółć gęślą jaźń 🙂".into()},
        }).await?;
        let inspect = guest.clone();
        let hierarchy = tokio::task::spawn_blocking(move||inspect.native_input_fixture(false)).await.map_err(|e|e.to_string())??;
        if !hierarchy.contains("Zażółć gęślą jaźń") {return Err("Native Unicode input missing from the hierarchy".into());}
        fs::write(trial.join("evidence/mcp-android-hierarchy.xml"),hierarchy).map_err(|e|e.to_string())?;
        let connection = runtime.connection().await?;
        let frame = connection.client().max_decoding_message_size(super::rpc::MAX_SCREENSHOT_BYTES+65536)
            .get_screenshot(connection.request("getScreenshot",crate::android_protocol::ImageFormat {format:0,..Default::default()})?)
            .await.map_err(|e|e.to_string())?.into_inner();
        let format = frame.format.ok_or("Missing screenshot format")?;
        if frame.image.len()>super::rpc::MAX_SCREENSHOT_BYTES || !frame.image.starts_with(b"\x89PNG\r\n\x1a\n") || format.width!=720 || format.height!=1280 {return Err("Unexpected native screenshot".into());}
        fs::write(trial.join("evidence/mcp-android.png"),&frame.image).map_err(|e|e.to_string())?;
        Router::submit(manager.clone(),id.clone(),generation.clone(),Request::Blur {lease:lease.clone()}).await?;
        if Router::submit(manager.clone(),id.clone(),generation.clone(),Request::Send {
            lease,sequence:2,event:Event::Text {action:TextAction::Commit,text:"STALE".into()},
        }).await.is_ok() {return Err("Released lease accepted input".into());}
        Ok(serde_json::json!({
            "root":root,"deviceId":id,"generation":generation,"state":"verified_and_stopped",
            "host":"macos-arm64","image":"system-images;android-36;default;arm64-v8a","profile":"small_phone",
            "nativeActor":true,"apkInstalled":true,"unicodeInput":true,"hierarchy":true,"nativePng":[format.width,format.height],"staleLeaseRejected":true,
            "scope":"Native P0 foundation; does not qualify a public MCP adapter, artifact staging or Tauri canvas"
        }))
    }.await;
    let stopped = manager.stop(&id, false).await;
    if manager
        .statuses()
        .unwrap()
        .iter()
        .any(|s| s.device_id == id && s.process_alive)
    {
        manager.stop(&id, true).await.unwrap();
    }
    manager.stop_private_adb_fixture().unwrap();
    stopped.unwrap();
    assert!(manager
        .statuses()
        .unwrap()
        .iter()
        .all(|s| s.device_id != id || !s.process_alive));
    let report = result.unwrap();
    fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{report}");
}
