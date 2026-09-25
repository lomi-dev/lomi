# Public repository audit

Snapshot date: September 25, 2026. Scope: all seven public repositories in
[lomi-dev](https://github.com/orgs/lomi-dev/repositories?type=public).

## Published metadata

All seven repository descriptions and topic lists were updated and verified
through a second GitHub API read. Four missing homepage links were added,
the website repository's homepage was normalized to `https://lomi.dev/`,
and the other two homepage links were retained.

| Repository                                                           | Description                                                                                                                                                                                     |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [lomi](https://github.com/lomi-dev/lomi)                             | Open-source desktop workspace for terminal-driven development, with native terminals, a code editor, browser previews, Git and AI chat. Built with Tauri and Rust for macOS, Linux and Windows. |
| [lomi-web](https://github.com/lomi-dev/lomi-web)                     | Source for lomi.dev: Lomi's website and newsletter signup, built with Astro, React and TypeScript. Currently a work-in-progress landing page.                                                   |
| [plugin-sdk](https://github.com/lomi-dev/plugin-sdk)                 | Public SDK for Lomi plugins: TypeScript APIs, the shared runtime contract, manifest validation and build helpers.                                                                               |
| [plugin-tools](https://github.com/lomi-dev/plugin-tools)             | Command-line tools for checking, building, diagnosing and packaging Lomi plugins, with testing helpers and the Workspace info example.                                                          |
| [create-lomi-plugin](https://github.com/lomi-dev/create-lomi-plugin) | Standalone generator for Lomi plugin projects, with panel, sidebar, command and theme templates and pinned SDK/CLI dependencies.                                                                |
| [simplevoice](https://github.com/lomi-dev/simplevoice)               | Local speech-to-text and voice typing for macOS, Linux and Windows, with offline models and optional cloud providers. Built with Tauri and Rust.                                                |
| [auth-frontend](https://github.com/lomi-dev/auth-frontend)           | Placeholder repository for the Lomi authentication frontend. No code or documentation has been published yet.                                                                                   |

The purpose of `auth-frontend` is inferred from its name. Its empty repository
does not establish an implementation plan or a technology stack. Its description
explicitly identifies that state.

The descriptions distinguish the plugin repositories' responsibilities:
`plugin-tools` owns the CLI, `create-lomi-plugin` owns the generator and templates,
and `plugin-sdk` owns the public contract.

## Settings snapshot

| Repository         | README                    | Repository license | Main branch protection | Dependabot alerts | GitHub Actions         |
| ------------------ | ------------------------- | ------------------ | ---------------------- | ----------------- | ---------------------- |
| lomi               | Present                   | Apache-2.0         | Disabled               | Disabled          | Enabled                |
| lomi-web           | Present                   | Missing            | Disabled               | Enabled           | Enabled                |
| plugin-sdk         | Present                   | Apache-2.0         | Disabled               | Enabled           | Intentionally disabled |
| plugin-tools       | Present                   | Apache-2.0         | Disabled               | Enabled           | Intentionally disabled |
| create-lomi-plugin | Present                   | Apache-2.0         | Disabled               | Enabled           | Intentionally disabled |
| simplevoice        | Present                   | Apache-2.0         | Disabled               | Disabled          | Enabled                |
| auth-frontend      | Missing; empty repository | Missing            | No branch exists       | Enabled           | Enabled                |

Secret scanning and secret scanning push protection were disabled in all seven
repositories. The rulesets API returned an empty list for each repository. No
`SECURITY.md` file was found in their inspected default-branch trees.

## Outstanding findings

1. **Repository protection.** The six existing `main` branches have no branch
   protection. Secret scanning and push protection are disabled everywhere;
   Dependabot alerts are disabled for `lomi` and `simplevoice`. These are settings
   findings, not evidence that secrets have leaked or that vulnerabilities exist.
2. **Website license.** `lomi-web` has no repository license file or license
   recognized by GitHub. The other projects' Apache-2.0 licenses were not applied
   to it automatically. The empty `auth-frontend` repository also has no license.
3. **Empty authentication repository.** GitHub reports an empty Git repository
   for `auth-frontend` and returns 404 for its README. Metadata has been added,
   but it still contains no code or documentation.
4. **Unavailable public plugin guide.** The generator README links to
   `https://github.com/lomi-dev/docs-app/blob/main/src/content/docs/plugins/quick-start.md`.
   That URL returned HTTP 404 without authentication. The About homepage links
   use public READMEs; the link inside the generator README remains to be fixed
   or replaced with a public guide.
5. **Unpublished plugin candidates.** The documentation describes SDK
   `1.1.0-alpha.1`, CLI `0.1.0-alpha.2`, and generator `0.1.0-alpha.3` candidates.
   The npm registry contained versions only through `1.1.0-alpha.0`,
   `0.1.0-alpha.1`, and `0.1.0-alpha.2`, respectively. The READMEs disclose that
   the newer candidates are unpublished; their installation commands are not
   yet an available registry installation path. No packages were published
   during this audit.
6. **Actions and validation.** Disabling Actions in the three plugin
   repositories agrees with their documented manual validation and publication
   process. Current workflows in `lomi` cover releases and the downloads badge;
   `simplevoice` workflows cover releases and AUR publication. No pull-request
   test workflow was found in the current trees. `lomi-web` has no GitHub Actions
   workflow; external hosting configuration was outside this audit's scope.
7. **Historical Simplevoice links.** Its README still references
   `MaciejKolerski/simplevoice`. The checked release URL redirects successfully
   to `lomi-dev/simplevoice`; these links can be normalized in a documentation
   update.

## Verification and scope

- Descriptions, homepages, and topics were read back after each update. A final
  organization inventory confirmed all seven public repositories were covered.
- `lomi.dev`, `www.lomi.dev`, and `simplevoice.app` returned HTTP 200. The public
  READMEs used as homepages also returned HTTP 200.
- Relative file and image links in the six existing root READMEs resolved to
  entries in the corresponding repository trees. This check did not validate
  every fragment identifier or every external link.
- The review covered root READMEs, file trees, package manifests, publication
  documentation, releases, workflows, and GitHub settings. It was an audit of
  repository presentation and configuration, not a complete code correctness
  or security audit.
- No application builds or tests were run for the metadata changes. Application
  code, licenses, workflows, and protection settings were not changed. Other
  inspected repository settings were compared with their original values.

The [JSON snapshot](repository-audit-2026-09-25.json) records descriptions,
homepages, and topics before and after the updates, settings responses, and link
and registry checks. These findings describe the dated snapshot; later changes
require a fresh check.
