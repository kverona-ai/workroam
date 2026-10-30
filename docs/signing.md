# Code signing

**English** · [简体中文](signing.zh-CN.md)

This guide is for maintainers. It covers how the release pipeline signs the files for each platform, how to get each GitHub secret, and how to check that signing works. The release process itself is in [release.md](release.md).

Every kind of signing is optional. When a platform is not configured, the release still publishes and that platform's files are left unsigned.

> **Private keys, certificates and passwords go only into GitHub secrets.** Add them on the **Secrets** tab of [Settings → Secrets and variables → Actions](https://github.com/kverona-ai/workroam/settings/secrets/actions). Values that are not secret, such as the SignPath organization ID, go on the **Variables** tab of the same page. Never post them in an issue, a pull request or a chat, and never commit them.

## Overview

| Platform | What is signed | How | You need |
| --- | --- | --- | --- |
| macOS | The desktop `.app` and `.dmg`, the `workroam` and `workroamd` CLI binaries, and `workroam-server` | Developer ID signing plus Apple notarization. The ticket is stapled to the `.app` and the `.dmg` | An Apple Developer Program account |
| Windows | Every `.exe`, including the binaries bundled in the desktop app, and the `.msi` and `-setup.exe` installers | Through SignPath, with the certificate that [SignPath Foundation](https://signpath.org) provides free to open source projects | An approved SignPath open source application |
| Linux | `SHA256SUMS`, which lists the checksum of every file | A detached GPG signature `SHA256SUMS.asc`, published with the public key `workroam-release-key.asc` | A GPG key that you generate locally |

Every release also gets GitHub build provenance (SLSA) for all files. That needs no setup.

Each Release run shows a **签名状态** (signing status) table in the summary of the Prepare job. It lists which kinds of signing are configured.

## macOS

You need an Apple Developer Program account, individual or organization. Only the Account Holder can create a Developer ID certificate.

### 1. Request a Developer ID Application certificate

1. On a Mac, open Keychain Access and choose **Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority**. Enter your email, choose "Saved to disk", and keep the `.certSigningRequest` file.
2. Open [Certificates](https://developer.apple.com/account/resources/certificates/add) and choose **Developer ID Application**. Do not choose Developer ID Installer or Apple Development. Upload the file from the previous step.
3. Download the `.cer` and double-click it to install it into the login keychain.

### 2. Export a .p12

In Keychain Access, find `Developer ID Application: …` under **My Certificates**. Its private key shows when you expand it. Right-click the certificate, choose Export, pick the `.p12` format and set a password. Then run:

```sh
base64 -i DeveloperID.p12 | pbcopy        # copy to the clipboard
security find-identity -v -p codesigning  # show the signing identity
```

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | The output of `base64` above |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | The full quoted string from `find-identity`, for example `Developer ID Application: Example Inc. (AB12CD34EF)` |

### 3. Notarization credentials (pick one)

**Option 1: an App Store Connect API key.** This is recommended, especially for an organization account. It is not tied to a personal Apple ID and is not affected by two-factor authentication.

1. Open [App Store Connect → Users and Access → Integrations → App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api). Generate a key under **Team Keys** with **Developer** access.
2. Download `AuthKey_<key ID>.p8`. It can only be downloaded once, so keep it somewhere safe.

| Secret | Value |
| --- | --- |
| `APPLE_API_KEY` | The key ID, 10 characters, for example `2X9R4HXF34` |
| `APPLE_API_ISSUER` | The Issuer ID at the top of the page, a UUID |
| `APPLE_API_PRIVATE_KEY` | The whole content of the `.p8` file, including the `-----BEGIN PRIVATE KEY-----` and `-----END PRIVATE KEY-----` lines |

**Option 2: an Apple ID with an app-specific password.**

| Secret | Value |
| --- | --- |
| `APPLE_ID` | The Apple ID email of the developer account |
| `APPLE_PASSWORD` | An app-specific password from [account.apple.com](https://account.apple.com) → Sign-In and Security → App-Specific Passwords. It is not the Apple ID password |
| `APPLE_TEAM_ID` | The 10-character Team ID from [Membership details](https://developer.apple.com/account#MembershipDetailsCard) |

When both are configured, the API key wins. With a certificate but no notarization credentials, files are signed but not notarized, and Gatekeeper still blocks the first launch.

### What the pipeline does

- **Desktop app.** Tauri signs the `.app` with the certificate, including the bundled `workroam` and `workroamd`. It then notarizes and staples the app, and signs the `.dmg`. The pipeline notarizes and staples the `.dmg` too. Finally it checks `codesign --verify --deep --strict`, `stapler validate` and `spctl --assess`.
- **CLI and `workroam-server`.** They are signed with the same certificate and the hardened runtime, then zipped and notarized. A bare executable cannot have a ticket stapled, so Gatekeeper looks up the notarization online.
- **Failures.** A failed notarization fails the job, and the log includes the detailed `notarytool log`.

## Windows (SignPath)

Workroam uses the free code signing certificate that [SignPath Foundation](https://signpath.org) provides to open source projects. The certificate belongs to SignPath Foundation, so Windows shows **SignPath Foundation** as the publisher. SignPath checks that every signed file was built from this repository by GitHub Actions.

> Since June 2023, new OV and EV code signing certificates must be kept in a hardware or cloud key store and cannot be exported as `.pfx` files. The pipeline no longer supports `.pfx`.

### 1. Apply

Submit an open source application at [signpath.org](https://signpath.org). The review looks at the following:

- The project uses an OSI-approved license (Workroam is Apache-2.0), the repository is public, and it has no closed-source components.
- The project is maintained, has released versions, and its features are documented.
- The project website, this repository, publishes a code signing policy. See [Code signing policy](#code-signing-policy) below.
- Everyone involved in signing uses two-factor authentication on GitHub and SignPath.

SignPath's application page and their reply are the authority on the exact conditions and steps.

### 2. Set up SignPath

After approval, SignPath creates an organization for the project at [app.signpath.io](https://app.signpath.io). Sign in, then:

1. **Project.** Check the project slug. The pipeline uses `workroam` by default.
2. **Trusted build system.** Under Trusted Build Systems in the organization settings, add **GitHub.com** and link it to the project.
3. **Artifact configuration.** Create an artifact configuration in the project, paste [`.github/signpath/artifact-configuration.xml`](../.github/signpath/artifact-configuration.xml), and make it the default. It applies an Authenticode signature to every `.exe` and `.msi` in the submitted archive.
4. **Signing policy.** Open source projects usually have `test-signing` and `release-signing`. The pipeline uses `release-signing` by default.
5. **API token.** Under Users, create a CI user and generate an API token. Add that user as a Submitter of the signing policy.

### 3. Add the GitHub secret and variables

| Name | Kind | Value |
| --- | --- | --- |
| `SIGNPATH_API_TOKEN` | Secret | The CI user's API token |
| `SIGNPATH_ORGANIZATION_ID` | Variable | The organization ID, a GUID shown in the organization settings and the URL |
| `SIGNPATH_PROJECT_SLUG` | Variable | The project slug. Defaults to `workroam`, so leave it unset if it matches |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Variable | The signing policy slug. Defaults to `release-signing` |
| `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | Variable | The artifact configuration slug. When unset, the project's default configuration is used |
| `SIGNPATH_WAIT_SECONDS` | Variable | How long to wait for signing, in seconds. Defaults to `7200` |

The pipeline submits files to SignPath only when both `SIGNPATH_API_TOKEN` and `SIGNPATH_ORGANIZATION_ID` are set.

### What the pipeline does

Each release sends SignPath two signing requests:

1. **windows-sign** submits the CLI `workroam.exe`, `workroamd.exe` and `workroam-server.exe`, plus the desktop app's `workroam-desktop.exe` and its bundled binaries.
2. **windows-bundle** builds the installers from the signed executables. It passes `--no-binary-patching`, so the signed main executable is not rewritten. Then **windows-installers** submits the `.msi` and `-setup.exe`.

A request under the `release-signing` policy needs an approver to approve it on the SignPath website, and then the job continues. If nobody approves within `SIGNPATH_WAIT_SECONDS`, the job fails, and you can re-run the failed jobs from Actions. After signing, the pipeline checks every file with `osslsigncode verify`.

### Code signing policy

This is the policy that the SignPath Foundation application asks the project to publish:

- Free code signing for the Windows builds is provided by [SignPath.io](https://about.signpath.io), with a certificate by [SignPath Foundation](https://signpath.org).
- Only files built by GitHub Actions from this repository's `main` branch or a release tag are submitted for signing.
- Committers and reviewers are the maintainers of this repository ([contributors](https://github.com/kverona-ai/workroam/graphs/contributors)). Every change is merged through a pull request, and `main` is protected.
- Approvers are the project maintainers, who approve each release signing request on SignPath.
- Privacy: Workroam connects to the network only where the user configured it or asked for it, such as the user's own S3 storage or server. It does not send information to any other system.

## Linux (GPG)

Linux packages need no purchased certificate. The pipeline signs `SHA256SUMS` with a GPG key. Users check the signature and then check their downloads against the checksums.

### 1. Generate a key

Generate a dedicated release signing key on your own computer, not in CI, and give it a passphrase:

```sh
gpg --quick-gen-key "Workroam Release <release@example.com>" ed25519 sign 3y
gpg --list-secret-keys --keyid-format long      # note the fingerprint
```

### 2. Export and store it

```sh
gpg --armor --export-secret-keys <fingerprint> | pbcopy    # on Linux: xclip -selection clipboard
```

| Secret | Value |
| --- | --- |
| `GPG_PRIVATE_KEY` | The whole exported text, including the `-----BEGIN PGP PRIVATE KEY BLOCK-----` line and the closing line |
| `GPG_PASSPHRASE` | The key's passphrase. Leave it unset if the key has none |

When it generates the key, GnuPG stores a revocation certificate under `~/.gnupg/openpgp-revocs.d/`. Keep it offline together with a backup of the private key. You can upload the public key to [keys.openpgp.org](https://keys.openpgp.org) so users can find it. Every release also includes `workroam-release-key.asc`.

### What the pipeline does

After it writes `SHA256SUMS`, the pipeline signs it into `SHA256SUMS.asc`, exports the public key as `workroam-release-key.asc`, and verifies the signature once. The "Verify" section of the release notes then gains a GPG command automatically.

## Check that signing works

1. Open [Actions → Release → Run workflow](https://github.com/kverona-ai/workroam/actions/workflows/release.yml), choose the `main` branch and **leave the tag empty**. This is a dry run that publishes nothing.
2. In the summary of the Prepare job, check that everything you configured shows ✅ in the signing status table.
3. If SignPath is configured, approve the two signing requests on the SignPath website.
4. When the run finishes, download `release-dist` from the run's Artifacts and check the files on each system:

```sh
# macOS
spctl --assess --type open --context context:primary-signature -vv Workroam-*.dmg
xcrun stapler validate Workroam-*.dmg
codesign -dv --verbose=2 workroam   # after unpacking the CLI archive
```

```powershell
# Windows. You can also right-click the file → Properties → Digital Signatures
Get-AuthenticodeSignature .\Workroam-*-setup.exe | Format-List Status, SignerCertificate
```

```sh
# Linux
gpg --import workroam-release-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
```

When these pass, release the next version as usual and signing applies automatically.

## When certificates and keys expire

- **Developer ID certificate.** It is valid for 5 years. Before it expires, repeat steps 1 and 2 and update `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY`. Older releases that were signed with a timestamp are not affected.
- **App Store Connect API key.** It does not expire, but it can be revoked on that page. After a revocation, update the three `APPLE_API_*` secrets.
- **GPG key.** It expires at the date set when you generated it. Extend it with `gpg --quick-set-expire <fingerprint> 3y` and export `GPG_PRIVATE_KEY` again.
- **SignPath certificate.** SignPath Foundation maintains it, so there is nothing to do.
