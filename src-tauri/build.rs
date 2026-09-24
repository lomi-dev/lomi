fn main() {
    assert!(
        !(std::env::var_os("CARGO_FEATURE_MCP_PROBE").is_some()
            && std::env::var("PROFILE").as_deref() == Ok("release")),
        "The mcp-probe qualification fixture must not enter a release build"
    );
    println!("cargo:rerun-if-changed=../packages/ai-runtime");
    println!("cargo:rerun-if-changed=../src/chat/provider-presets.ts");
    println!("cargo:rerun-if-changed=../scripts/prepare-ai-runtime.mjs");
    let target = std::env::var("TARGET").expect("missing Cargo target");
    println!("cargo:rustc-env=LOMI_AI_TARGET={target}");
    let mut prepare = std::process::Command::new("node");
    prepare
        .arg("../scripts/prepare-ai-runtime.mjs")
        .env("TARGET", &target);
    if std::env::var_os("CARGO_FEATURE_CHAT_PROBE").is_some()
        || std::env::var_os("CARGO_FEATURE_MCP_PROBE").is_some()
    {
        prepare.arg("--fixture");
    }
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
    tauri_build::try_build(tauri_build::Attributes::new().plugin(
        "browser",
        tauri_build::InlinedPlugin::new().commands(&["signal"]),
    ))
    .expect("failed to build Tauri permissions");
}
