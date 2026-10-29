use lomi_remote_protocol::ipc::{LocalAuthClientId, LocalAuthOrigin, LocalRelayEndpoint};

fn main() {
    configure_host_auth_bootstrap();
    println!("cargo:rerun-if-changed=../packages/ai-runtime");
    println!("cargo:rerun-if-changed=../src/chat/provider-presets.ts");
    println!("cargo:rerun-if-changed=../scripts/prepare-ai-runtime.mjs");
    let target = std::env::var("TARGET").expect("missing Cargo target");
    println!("cargo:rustc-env=LOMI_AI_TARGET={target}");
    let mut prepare = std::process::Command::new("node");
    prepare
        .arg("../scripts/prepare-ai-runtime.mjs")
        .env("TARGET", &target);
    assert!(
        prepare
            .status()
            .expect("Install Node and run pnpm install before building")
            .success(),
        "AI runtime preparation failed"
    );

    {
        println!("cargo:rerun-if-changed=android-proto/emulator_controller.proto");
        let mut config = tonic_prost_build::Config::new();
        config.protoc_executable(protoc_bin_vendored::protoc_bin_path().unwrap());
        config.bytes([".android.emulation.control.Image.image"]);
        tonic_prost_build::configure()
            .build_server(false)
            .compile_with_config(
                config,
                &["android-proto/emulator_controller.proto"],
                &["android-proto"],
            )
            .expect("failed to compile the pinned Android emulator protocol");
    }
    let mut attributes = tauri_build::Attributes::new().plugin(
        "browser",
        tauri_build::InlinedPlugin::new().commands(&["signal"]),
    );
    if target.ends_with("-windows-msvc") {
        // Native test executables do not receive Tauri's resource manifest.
        // Embed the same manifest for tests and the application through the linker.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("failed to build Tauri permissions");
}

fn configure_host_auth_bootstrap() {
    const VARIABLES: [&str; 5] = [
        "LOMI_HOST_IDENTITY_ORIGIN",
        "LOMI_HOST_REMOTE_API_ORIGIN",
        "LOMI_HOST_RELAY_ENDPOINT",
        "LOMI_HOST_CLIENT_ID",
        "LOMI_HOST_REMOTE_WEB_ORIGIN",
    ];
    let values: Vec<Option<String>> = VARIABLES
        .iter()
        .map(|name| {
            println!("cargo:rerun-if-env-changed={name}");
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .collect();
    let configured = values.iter().filter(|value| value.is_some()).count();
    if std::env::var_os("CARGO_FEATURE_REMOTE_HOST").is_none() {
        println!("cargo:rustc-env=LOMI_HOST_AUTH_BOOTSTRAP_CONFIGURED=0");
        return;
    }
    if configured != 0 && configured != VARIABLES.len() {
        panic!("Set all five Lomi host authentication bootstrap variables together.");
    }
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    if release && configured != VARIABLES.len() {
        panic!("Release builds require the five public Lomi host authentication bootstrap values.");
    }

    if configured == VARIABLES.len() {
        let values: Vec<String> = values.into_iter().map(Option::unwrap).collect();
        LocalAuthOrigin::try_from(values[0].clone()).expect("LOMI_HOST_IDENTITY_ORIGIN is invalid");
        LocalAuthOrigin::try_from(values[1].clone())
            .expect("LOMI_HOST_REMOTE_API_ORIGIN is invalid");
        LocalRelayEndpoint::try_from(values[2].clone())
            .expect("LOMI_HOST_RELAY_ENDPOINT is invalid");
        LocalAuthClientId::try_from(values[3].clone()).expect("LOMI_HOST_CLIENT_ID is invalid");
        LocalAuthOrigin::try_from(values[4].clone())
            .expect("LOMI_HOST_REMOTE_WEB_ORIGIN is invalid");
        for (key, value) in VARIABLES.iter().zip(values) {
            println!("cargo:rustc-env={key}={value}");
        }
        println!("cargo:rustc-env=LOMI_HOST_AUTH_BOOTSTRAP_CONFIGURED=1");
    } else {
        println!("cargo:rustc-env=LOMI_HOST_AUTH_BOOTSTRAP_CONFIGURED=0");
    }
}
