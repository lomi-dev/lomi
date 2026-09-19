import type { Catalog, Package, Snapshot, Profile } from "./types";

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

type ImageChoice = { id: string; revision: string; image?: Package["image"] };

export function imageInfo(image: Pick<ImageChoice, "id" | "image">) {
  const parts = image.id.split(";");
  const [major, minor = 0] = (parts[1]?.replace("android-", "") ?? "")
    .split(".")
    .map(Number);
  return (
    image.image ?? {
      api: major,
      minorApi: minor,
      tag: parts[2] ?? "",
      abi: parts[3] ?? "",
    }
  );
}

export function androidVersion(api: number, minor = 0) {
  const releases: Record<number, string> = {
    26: "8.0",
    27: "8.1",
    28: "9",
    29: "10",
    30: "11",
    31: "12",
    32: "12L",
    33: "13",
    34: "14",
    35: "15",
    36: "16",
    37: "17",
  };
  const level = `API ${api}${minor ? `.${minor}` : ""}`;
  return releases[api] ? `Android ${releases[api]} (${level})` : level;
}

export function imageFamily(image: Pick<ImageChoice, "id" | "image">) {
  const tag = imageInfo(image).tag;
  return tag.startsWith("google_apis_playstore")
    ? "play"
    : tag.startsWith("google_apis")
      ? "google"
      : "aosp";
}

export function imageTitle(image: Pick<ImageChoice, "id" | "image">) {
  const info = imageInfo(image);
  const label = { play: "Google Play", google: "Google APIs", aosp: "AOSP" }[
    imageFamily(image)
  ];
  return `${androidVersion(info.api, info.minorApi)} · ${label}${info.tag.endsWith("_ps16k") ? " · 16 KB" : ""}`;
}

export function imageLabel(image: ImageChoice) {
  const info = imageInfo(image);
  return `${imageTitle(image)} · ${info.abi} · r${image.revision}`;
}

export function imageDescription(image: Pick<ImageChoice, "id" | "image">) {
  const family = imageFamily(image);
  return (
    (family === "play"
      ? "Includes Play Store and Google services."
      : family === "google"
        ? "Google services for testing; no Play Store."
        : "Open-source Android with basic apps; no Google services.") +
    (imageInfo(image).tag.endsWith("_ps16k")
      ? " Uses 16 KB memory pages; apps with native libraries must support this page size."
      : "")
  );
}

export function compareImages(a: ImageChoice, b: ImageChoice) {
  const first = imageInfo(a),
    second = imageInfo(b);
  const rank = (image: ImageChoice) =>
    ({ play: 0, google: 1, aosp: 2 })[imageFamily(image)];
  return (
    second.api - first.api ||
    second.minorApi - first.minorApi ||
    rank(a) - rank(b) ||
    Number(first.tag.endsWith("_ps16k")) -
      Number(second.tag.endsWith("_ps16k")) ||
    b.revision.localeCompare(a.revision, undefined, { numeric: true }) ||
    a.id.localeCompare(b.id)
  );
}

export function compatibleProfile(
  profile: Profile,
  image: Pick<ImageChoice, "id" | "image">,
) {
  const info = imageInfo(image);
  return (
    info.api > profile.minApi ||
    (info.api === profile.minApi && info.minorApi >= profile.minMinorApi)
  );
}

export function compareProfiles(a: Profile, b: Profile) {
  return (
    b.minApi - a.minApi ||
    b.minMinorApi - a.minMinorApi ||
    a.width * a.height - b.width * b.height ||
    b.name.localeCompare(a.name, undefined, { numeric: true })
  );
}

/** Select only requested tools/images and their declared dependencies. */
export function installationSelection(
  catalog: Catalog,
  snapshot: Snapshot,
  ids: string[],
  repair: boolean,
) {
  const selected = new Map<string, Package>();
  const pending = [...ids];
  while (pending.length) {
    const id = pending.shift()!;
    if (selected.has(id)) continue;
    const installed = snapshot.packages?.packages[id];
    const pkg = catalog.packages.find(
      (candidate) =>
        candidate.id === id &&
        (!repair || !installed || candidate.revision === installed.revision),
    );
    if (!pkg)
      throw new Error(
        repair && installed
          ? `Revision ${installed.revision} of ${id} is no longer available from the provider. The installed files have been preserved.`
          : `No compatible package is available for ${id}. Refresh the catalog and retry.`,
      );
    const users =
      snapshot.devices?.devices.filter((device) => device.image === id) ?? [];
    if (users.length && installed && pkg.revision !== installed.revision)
      throw new Error(
        `This image is used by ${users.map((device) => device.name).join(", ")}. Its revision cannot change; choose a different image for a new device.`,
      );
    selected.set(id, pkg);
    for (const dependency of pkg.dependencies) {
      const current = snapshot.packages?.packages[dependency.id];
      const version = (value: string) => value.split(".").map(Number);
      const enough =
        current &&
        (!dependency.minimumRevision ||
          version(dependency.minimumRevision).every((value, index, minimum) => {
            const actual = version(current.revision);
            for (let before = 0; before < index; before++) {
              if ((actual[before] ?? 0) !== minimum[before])
                return (actual[before] ?? 0) > minimum[before];
            }
            return (actual[index] ?? 0) >= value;
          }));
      if (!enough) pending.push(dependency.id);
    }
    if (selected.size > 16)
      throw new Error(
        "This selection requires too many packages. Install the tools first.",
      );
  }
  return [...selected.values()].map(({ id, revision }) => ({ id, revision }));
}
