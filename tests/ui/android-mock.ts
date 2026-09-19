import type { Page } from "@playwright/test";

export async function mockAndroid(page: Page, prepared = false) {
  await page.addInitScript(
    ({ prepared }) => {
      const desktop = window as any;
      const deviceId = "12345678-1234-4567-8123-123456789abc";
      const generation = "87654321-4321-4765-8321-cba987654321";
      const imageId = "system-images;android-36;default;arm64-v8a";
      const requiredTools = [
        "emulator",
        "platform-tools",
        "cmdline-tools;23.0",
      ];
      const packages = [...requiredTools, imageId].map((id) => ({
        id,
        revision: id === imageId ? "2" : "37.1.11",
        name: id,
        size: 1000000,
        license: "android-sdk-license",
        url: "https://example.invalid/fixture.zip",
        sha1: "a".repeat(40),
        dependencies: [],
        image:
          id === imageId
            ? { api: 36, minorApi: 0, tag: "default", abi: "arm64-v8a" }
            : null,
      }));
      const device = {
        id: deviceId,
        name: "Test phone",
        image: imageId,
        imageRevision: 2,
        profile: "small_phone",
        inputBridge: true,
        hardware: {
          ramMib: 2560,
          cpuCount: 2,
          dataGib: 4,
          gpu: "auto",
          quickBoot: false,
        },
      };
      const state = {
        host: "darwin_arm64",
        qualified: true,
        acceleration: {
          available: true,
          backend: "Hypervisor.framework",
          action: null,
        },
        sdkPath: "/isolated/android/sdk",
        adbPath: prepared ? "/isolated/android/sdk/platform-tools/adb" : null,
        toolchainReady: prepared,
        toolchainUpdateAvailable: !prepared,
        preferences: {
          version: 1,
          revision: 0,
          defaultDeviceId: prepared ? deviceId : null,
        },
        devices: { version: 1, revision: 0, devices: prepared ? [device] : [] },
        packages: {
          version: 1,
          revision: 0,
          packages: Object.fromEntries(
            (prepared ? packages : []).map((pkg) => [
              pkg.id,
              { id: pkg.id, revision: pkg.revision, archiveSha1: pkg.sha1 },
            ]),
          ),
        },
        profiles: prepared
          ? [
              {
                id: "small_phone",
                name: "Small Phone",
                width: 720,
                height: 1280,
                dpi: 320,
                minApi: 26,
                minMinorApi: 0,
              },
            ]
          : [],
        errors: {},
        statuses: [] as any[],
        operation: null as any,
        streams: [] as any[],
        rollbacks: [],
        recovery: [],
        requiredTools,
        toolchain: {
          qualified: true,
          cli: {
            version: "1.0.16261425",
            size: 100,
            url: "https://example.invalid/cli",
            sha256: "b".repeat(64),
          },
          java: {
            version: "21.0.12.1+1",
            size: 100,
            url: "https://example.invalid/java",
            sha256: "b".repeat(64),
          },
        },
      };
      const catalog = {
        revision: "catalog-1",
        packages,
        licenses: [
          {
            id: "android-sdk-license",
            text: "Fixture terms shown in full before consent. This mock does not accept a provider license.",
            digest: "license-digest",
          },
        ],
      };
      let epoch = 0;
      const live = new Map<number, any>();
      let lease = "";
      let selectedPackages: string[] = [];
      desktop.__androidTest = {
        state,
        catalog,
        starts: 0,
        startRequests: 0,
        stops: 0,
        subscribers: 0,
        live,
        stopDelay: 0,
        input: [] as any[],
        generation,
        deviceId,
        failStop: false,
      };
      const changed = () =>
        desktop.__nativeTest.emitEvent("android-changed", { kind: "metadata" });
      desktop.__androidInvoke = async (command: string, args: any) => {
        if (command === "android_state") return structuredClone(state);
        if (command === "android_storage")
          return {
            freeBytes: 80 * 1024 ** 3,
            growthReserveBytes: 4 * 1024 ** 3,
            directories: {
              sdk: {
                allocatedBytes: prepared ? 3000000000 : 0,
                logicalBytes: prepared ? 3000000000 : 0,
              },
            },
          };
        if (command === "android_catalog") return structuredClone(catalog);
        if (command === "android_setup_context")
          return desktop.__androidTest.setup ?? null;
        if (command === "android_request_open") return "open-request";
        if (command === "android_prepare_setup")
          return (desktop.__androidTest.setup = {
            requestId: crypto.randomUUID(),
            workspaceId: args.workspaceId,
            panelId: args.panelId,
          });
        if (command === "android_install_plan") {
          selectedPackages = args.packages.map((pkg: any) => pkg.id);
          return {
            id: "reviewed-plan",
            catalogRevision: catalog.revision,
            packages: packages.filter((pkg) =>
              args.packages.some((item: any) => item.id === pkg.id),
            ),
            licenses: catalog.licenses,
            bootstrap: args.prepareTools ? state.toolchain : null,
            downloadBytes: 3000200,
          };
        }
        if (command === "android_install") {
          if (args.accepted.join() !== "license-digest")
            throw new Error("Provider consent is required");
          state.operation = {
            operationId: crypto.randomUUID(),
            packageIds: selectedPackages,
            phase: "running",
            stage: "Downloading verified components",
            received: 250000,
            total: 1000000,
            error: null,
            deviceId: null,
          };
          await changed();
          return state.operation;
        }
        if (command === "android_cancel_operation") {
          if (desktop.__androidTest.cancelError)
            throw new Error(desktop.__androidTest.cancelError);
          if (state.operation.operationId !== args.operationId)
            throw new Error("Stale operation");
          state.operation.phase = "cancelling";
          await changed();
          await new Promise((resolve) =>
            setTimeout(resolve, desktop.__androidTest.cancelDelay ?? 0),
          );
          state.operation.phase = "cancelled";
          state.operation.stage = "Installation cancelled";
          await changed();
          return;
        }
        if (command === "save_android_preferences") {
          state.preferences = {
            ...args.data,
            revision: state.preferences.revision + 1,
          };
          await changed();
          return structuredClone(state.preferences);
        }
        if (command === "android_manage_device") {
          if (args.action.type === "delete")
            state.devices.devices = state.devices.devices.filter(
              (device) => device.id !== args.action.deviceId,
            );
          if (args.action.type === "update")
            state.devices.devices = state.devices.devices.map((device) =>
              device.id === args.action.device.id ? args.action.device : device,
            );
          if (args.action.type === "create")
            state.devices.devices.push({
              ...device,
              ...args.action.draft,
              id: crypto.randomUUID(),
            });
          state.devices.revision++;
          state.operation = {
            operationId: "manage-1",
            packageIds: [],
            phase: "succeeded",
            stage: "Device updated",
            received: 0,
            total: 0,
            error: null,
            deviceId: device.id,
          };
          await changed();
          return state.operation;
        }
        if (command === "android_start") {
          desktop.__androidTest.startRequests++;
          if (
            !state.statuses.some(
              (status) =>
                status.deviceId === args.deviceId && status.processAlive,
            )
          )
            desktop.__androidTest.starts++;
          if (desktop.__androidTest.holdStart)
            await new Promise<void>((resolve, reject) => {
              desktop.__androidTest.finishStart = resolve;
              desktop.__androidTest.failStart = reject;
            }).finally(() => {
              desktop.__androidTest.finishStart = undefined;
              desktop.__androidTest.failStart = undefined;
            });
          const status = {
            deviceId: args.deviceId,
            generation,
            phase: "running",
            processAlive: true,
            serial: "emulator-5588",
            display: desktop.__androidTest.display ?? [720, 1280],
            error: null,
          };
          state.statuses = [
            ...state.statuses.filter((item) => item.deviceId !== args.deviceId),
            status,
          ];
          await desktop.__nativeTest.emitEvent("android-changed", {
            kind: "status",
            value: status,
          });
          return status;
        }
        if (command === "android_stop") {
          desktop.__androidTest.stops++;
          desktop.__androidTest.failStart?.(
            new Error("Android start was cancelled by Stop"),
          );
          await new Promise((resolve) =>
            setTimeout(resolve, desktop.__androidTest.stopDelay),
          );
          if (desktop.__androidTest.failStop)
            throw new Error("Phone is still running. Retry Stop.");
          const status = {
            deviceId: args.deviceId,
            generation,
            phase: "stopped",
            processAlive: false,
            serial: null,
            display: null,
            error: null,
          };
          state.statuses = [status];
          await desktop.__nativeTest.emitEvent("android-changed", {
            kind: "status",
            value: status,
          });
          return status;
        }
        if (command === "android_subscribe_frames") {
          if (desktop.__androidTest.holdScreen)
            await new Promise<void>((resolve) => {
              desktop.__androidTest.connectScreen = resolve;
            });
          const current = ++epoch;
          live.clear();
          live.set(current, args.frames);
          desktop.__androidTest.subscribers++;
          const width = args.size.width,
            height = args.size.height;
          const bytes = new ArrayBuffer(
            Math.max(1024, 68 + width * height * 4),
          );
          const header = new DataView(bytes);
          header.setUint32(0, 0x50414253, true);
          header.setUint32(4, 4, true);
          const uuid = generation.replaceAll("-", "").match(/../g)!;
          uuid.forEach((pair, index) =>
            header.setUint8(8 + index, parseInt(pair, 16)),
          );
          header.setBigUint64(24, BigInt(current), true);
          header.setBigUint64(32, 1n, true);
          header.setUint32(48, width, true);
          header.setUint32(52, height, true);
          header.setUint32(64, width * height * 4, true);
          new Uint8Array(bytes, 68).fill(90);
          args.frames.onmessage(bytes);
          state.streams = [
            {
              deviceId: args.deviceId,
              generation,
              epoch: current,
              phase: "streaming",
              error: null,
              grpcFrames: 1,
              grpcBytes: width * height * 4,
              ipcFrames: 1,
              ipcBytes: bytes.byteLength,
            },
          ];
          await desktop.__nativeTest.emitEvent("android-changed", {
            kind: "stream",
            value: state.streams[0],
          });
          return current;
        }
        if (command === "android_unsubscribe_frames") {
          live.delete(args.epoch);
          return;
        }
        if (command === "android_ack_frame") return;
        if (
          ["android_install_apk", "android_save_screenshot"].includes(command)
        ) {
          if (desktop.__androidTest.holdFileAction)
            await new Promise<void>((resolve) => {
              desktop.__androidTest.finishFileAction = resolve;
            });
          return;
        }
        if (command === "android_input") {
          if (args.input.type === "focus") lease = crypto.randomUUID();
          if (args.input.type === "send")
            desktop.__androidTest.input.push(args.input.event);
          return { lease, sequence: args.input.sequence ?? 0 };
        }
        if (
          [
            "android_open_result",
            "android_maintenance",
            "android_export_diagnostics",
          ].includes(command)
        )
          return;
        throw new Error(`Unexpected Android mock command: ${command}`);
      };
    },
    { prepared },
  );
}
