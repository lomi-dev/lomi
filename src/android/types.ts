export interface Preferences {
  version: 1;
  revision: number;
  defaultDeviceId: string | null;
}
export interface Hardware {
  ramMib: number;
  cpuCount: number;
  dataGib: number;
  gpu: "auto" | "host" | "software";
  quickBoot: boolean;
}
export interface Device {
  id: string;
  name: string;
  image: string;
  imageRevision: number;
  profile: string;
  hardware: Hardware;
  inputBridge: boolean;
}
export interface Devices {
  version: 1;
  revision: number;
  devices: Device[];
}
export interface Profile {
  id: string;
  name: string;
  width: number;
  height: number;
  dpi: number;
  minApi: number;
  minMinorApi: number;
}
export interface Package {
  id: string;
  revision: string;
  name: string;
  license: string;
  url: string;
  size: number;
  sha1: string;
  image: { api: number; minorApi: number; tag: string; abi: string } | null;
  dependencies: { id: string; minimumRevision: string | null }[];
}
export interface Installed {
  id: string;
  revision: string;
  archiveSha1: string;
}
export interface Manifest {
  version: 1;
  revision: number;
  packages: Record<string, Installed>;
}
export interface License {
  id: string;
  text: string;
  digest: string;
}
export interface Catalog {
  revision: string;
  packages: Package[];
  licenses: License[];
}
export interface Download {
  version: string;
  url: string;
  size: number;
  sha256: string;
}
export interface Plan {
  id: string;
  catalogRevision: string;
  packages: Package[];
  licenses: License[];
  bootstrap: { cli: Download; java: Download; qualified: boolean } | null;
  downloadBytes: number;
}
export interface Progress {
  operationId: string;
  packageIds: string[];
  phase: "running" | "cancelling" | "succeeded" | "cancelled" | "failed";
  stage: string;
  received: number;
  total: number;
  error: string | null;
  deviceId: string | null;
}
export interface Status {
  deviceId: string;
  generation: string | null;
  phase: "stopped" | "starting" | "booting" | "running" | "stopping" | "failed";
  processAlive: boolean;
  serial: string | null;
  error: string | null;
  display: [number, number] | null;
}
export interface StreamStatus {
  deviceId: string;
  generation: string;
  epoch: number;
  phase: "connecting" | "streaming" | "sleeping" | "hidden" | "disconnected";
  error: string | null;
  grpcFrames: number;
  grpcBytes: number;
  ipcFrames: number;
  ipcBytes: number;
}
export interface Snapshot {
  host: string;
  qualified: boolean;
  acceleration: { available: boolean; backend: string; action: string | null };
  sdkPath: string;
  adbPath: string | null;
  toolchainReady: boolean;
  toolchainUpdateAvailable: boolean;
  preferences: Preferences | null;
  devices: Devices | null;
  packages: Manifest | null;
  profiles: Profile[];
  errors: Record<string, string>;
  statuses: Status[];
  operation: Progress | null;
  streams: StreamStatus[];
  rollbacks: Installed[];
  recovery: {
    file: "preferences" | "devices";
    digest: string;
    backupRevision: number | null;
  }[];
  requiredTools: string[];
  toolchain: NonNullable<Plan["bootstrap"]>;
}
export type Changed =
  | { kind: "metadata" }
  | { kind: "operation"; value: Progress }
  | { kind: "status"; value: Status }
  | { kind: "stream"; value: StreamStatus }
  | { kind: "inputError"; value: string };

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const control = /[\u0000-\u001f\u007f-\u009f]/;
const bytes = (value: string) => new TextEncoder().encode(value).length;
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).some((key) => !keys.includes(key)) ||
    keys.some((key) => !(key in value))
  )
    throw new Error(
      "Unsupported Android metadata. The existing file has been preserved.",
    );
  return value as Record<string, unknown>;
}
function integer(value: unknown, min: number, max: number): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= min &&
    value <= max
  );
}
function version(value: Record<string, unknown>) {
  if (
    value.version !== 1 ||
    !integer(value.revision, 0, Number.MAX_SAFE_INTEGER)
  )
    throw new Error(
      "Unsupported Android metadata version or revision. Open recovery in Android settings.",
    );
}
export function parsePreferences(value: unknown): Preferences {
  const data = record(value, ["version", "revision", "defaultDeviceId"]);
  version(data);
  if (
    data.defaultDeviceId !== null &&
    (typeof data.defaultDeviceId !== "string" ||
      !uuid.test(data.defaultDeviceId))
  )
    throw new Error(
      "Invalid default Android device. The file has been preserved.",
    );
  return data as unknown as Preferences;
}
export function parseDevice(value: unknown): Device {
  const data = record(value, [
    "id",
    "name",
    "image",
    "imageRevision",
    "profile",
    "hardware",
    "inputBridge",
  ]);
  const hardware = record(data.hardware, [
    "ramMib",
    "cpuCount",
    "dataGib",
    "gpu",
    "quickBoot",
  ]);
  const parts = typeof data.image === "string" ? data.image.split(";") : [];
  const component = (value: string) =>
    value.length <= 100 && /^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(value);
  const major = parts[1]?.slice(8).split(/[.-]/)[0] ?? "";
  if (
    typeof data.id !== "string" ||
    !uuid.test(data.id) ||
    typeof data.name !== "string" ||
    data.name.trim() !== data.name ||
    !data.name ||
    Array.from(data.name).length > 80 ||
    control.test(data.name) ||
    parts.length !== 4 ||
    parts[0] !== "system-images" ||
    !parts[1].startsWith("android-") ||
    !component(parts[1].slice(8)) ||
    !/^\d+$/.test(major) ||
    !integer(Number(major), 26, 999) ||
    !component(parts[2]) ||
    !["arm64-v8a", "x86_64"].includes(parts[3]) ||
    !integer(data.imageRevision, 1, 0xffffffff) ||
    typeof data.profile !== "string" ||
    !data.profile ||
    data.profile.startsWith("-") ||
    bytes(data.profile) > 160 ||
    control.test(data.profile) ||
    typeof data.inputBridge !== "boolean" ||
    typeof hardware.quickBoot !== "boolean" ||
    !["auto", "host", "software"].includes(hardware.gpu as string) ||
    !integer(hardware.ramMib, 512, 32768) ||
    !integer(hardware.cpuCount, 1, 32) ||
    !integer(hardware.dataGib, 2, 128)
  )
    throw new Error(
      "Invalid Android device configuration. No changes were saved.",
    );
  return data as unknown as Device;
}
export function parseDevices(value: unknown): Devices {
  const data = record(value, ["version", "revision", "devices"]);
  version(data);
  if (!Array.isArray(data.devices))
    throw new Error(
      "Invalid Android devices file. Open recovery in Android settings.",
    );
  const devices = data.devices.map(parseDevice);
  if (new Set(devices.map((device) => device.id)).size !== devices.length)
    throw new Error(
      "Duplicate Android device ID. The file has been preserved.",
    );
  return { version: 1, revision: data.revision as number, devices };
}
