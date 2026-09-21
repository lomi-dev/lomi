import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

export async function pluginSDKSpec(
  root,
  archive = process.env.LOMI_SDK_TARBALL,
) {
  if (archive) return `file:${resolve(archive)}`;
  const metadata = JSON.parse(
    await readFile(resolve(root, "package.json"), "utf8"),
  );
  const spec = metadata.dependencies["@lomi-dev/plugin-sdk"];
  if (!spec)
    throw new Error("The application must declare @lomi-dev/plugin-sdk.");
  return spec.startsWith("file:")
    ? `file:${resolve(root, spec.slice(5))}`
    : spec;
}
