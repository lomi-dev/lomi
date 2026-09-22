# Packed Lomi plugin SDK

`lomi-dev-plugin-sdk-1.1.0-alpha.1.tgz` is the installable package built from
[lomi-dev/plugin-sdk at 01070882e3112dc7035cd0baea54303f3c64b966](https://github.com/lomi-dev/plugin-sdk/tree/01070882e3112dc7035cd0baea54303f3c64b966).
The archive includes its Apache-2.0 license, compiled exports and public types.

This artifact supplies the unpublished Lomi runtime contract to clean application
installs. The root application and context-plugin fixture reference the same file;
pnpm records its integrity in the lockfile. `release.json` records the source and
archive hashes, and `pnpm sdk:verify` checks the archive's integrity.
No sibling checkout is required.

Make SDK changes in its standalone repository, build and qualify a new archive,
then update both dependencies, this artifact, its report and the lockfile together.
Once the compatible version is published to npm, prefer the exact registry version.
