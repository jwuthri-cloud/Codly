# Releasing Codly

How a version of Codly becomes a download. This page is for the person who releases; what a user
does with a download is in the [README](../README.md#install).

A release is two files, `Codly_<version>_aarch64.dmg` (Apple Silicon) and `Codly_<version>_x64.dmg`
(Intel), plus `SHA256SUMS.txt`, on the repository's Releases page. The workflow
`.github/workflows/release.yml` builds them on GitHub's macOS machines when you push a tag
`v<version>`, and puts them in a **draft** release. Nothing is public until you publish the draft.

## Before the first release

The workflow was written from the Tauri and GitHub Actions documentation. It has not run yet, and
it could not be tested before it was merged. Run it once by hand (see
[Try the build without a release](#try-the-build-without-a-release)) and fix what fails before you
push the first tag. A run by hand does not reach the last job, the one that writes the draft
release: that job runs for the first time on your first tag. Remove this section after the first
release.

## The version

The version has one source: `version` under `[workspace.package]` in the root `Cargo.toml`.
The app bundle (Finder, the About window), `codly --version`, the first line of the diagnostics
report and the names of the `.dmg` files all come from it. `src-tauri/tauri.conf.json` and
`package.json` have no version on purpose, and a test (`the_version_has_one_source`) fails if one
comes back.

To change it:

1. Edit `version` under `[workspace.package]` in `Cargo.toml`.
2. Run `cargo check`. It writes the new version of `codly` and `codly-core` to `Cargo.lock` (or
   change the `version` line of those two entries by hand). CI builds with `--locked` and fails
   when `Cargo.lock` is out of date.
3. Commit `Cargo.toml` and `Cargo.lock`.

## Make a release

1. Set the version (above) and merge it to `main`. Wait until CI is green on that commit.
2. Tag that commit and push the tag. The tag must be `v` plus the version:

   ```bash
   git switch main && git pull
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. Open the repository's Actions tab and watch the run named Release. It builds the two apps in
   parallel; a build from nothing takes a while, and notarization can add more.
4. When the run is green, open Releases. The draft "Codly 0.2.0" has the two `.dmg` files,
   `SHA256SUMS.txt` and notes made from the merged pull requests.
5. [Test the draft](#test-a-draft).
6. Edit the notes if you want to, then choose Publish release.

When something goes wrong:

- **The tag and the version differ.** The run stops at "Check the tag is the app's version".
  Delete the tag (`git push --delete origin v0.2.0`, then `git tag -d v0.2.0`), fix the version
  and tag again.
- **A build fails.** Fix it on `main`, delete the tag, and tag the new commit. If the cause was
  not the code (a network error, Apple's service), choose Re-run failed jobs instead.
- **A run for a tag that already has a draft** (Re-run all jobs on the tag's run) replaces the
  files of the draft and keeps its notes. Delete the draft first if you want new notes. The line
  "This build is not signed" is part of the notes, and it is written only when the draft is made.
  So after you turn signing on or off, delete the draft before the re-run, or edit that line by
  hand: the run prints a warning that says which of the two the new files are.
- **A run for a tag that is already published** stops: a published release is never changed. Use
  a new version.

## Try the build without a release

Actions → Release → Run workflow, on any branch or tag. This builds the two `.dmg` files and keeps
them as artifacts of the run (at the bottom of the run's page). It makes no tag and no release,
and it does not change the draft of a tag you run it on: only a pushed tag writes a release. Use
it to test a change to the workflow or a new set of signing secrets.

## Test a draft

Do this on a Mac that did not build Codly, for each of the two files if you can. The Intel file
also runs on an Apple Silicon Mac that has Rosetta.

1. Download the `.dmg` and `SHA256SUMS.txt` from the draft **with a browser**. A browser marks the
   file as downloaded from the internet, and only then does macOS check the app as it will for a
   user. A file fetched with `gh release download` or `curl` skips that check.
2. Check the file: `shasum -a 256 -c --ignore-missing SHA256SUMS.txt` prints `OK`.
3. Open the `.dmg`, drag Codly to Applications, and open it.
   - Signed build: it opens after the usual "downloaded from the internet" question.
   - Unsigned build: macOS stops it. Follow the steps in the README (System Settings → Privacy &
     Security → Open Anyway). If macOS says the app "is damaged", the signature is broken: do
     not publish, and look at the "Check the signature" step of the run.
4. In a terminal, `/Applications/Codly.app/Contents/MacOS/codly --version` prints the version of
   the tag. Settings → General → Troubleshooting → Collect diagnostics starts with the same.
5. Use it: open a repository, send the Lead a message, open a terminal, and try dictation (macOS
   must ask for the microphone). A release build runs with Apple's hardened runtime, which a build
   from `scripts/install-app.sh` does not, so a feature can fail here and work on your own build.
6. Signed build only: `spctl --assess --type execute --verbose=2 /Applications/Codly.app` prints
   `accepted` and `source=Notarized Developer ID`.

## Signing and notarization

Signing is a switch. The workflow reads six repository secrets:

- All six exist: the app and the `.dmg` are signed with your Developer ID, Apple notarizes the
  app, and macOS opens it without a warning.
- None exists: the build is not signed with a Developer ID
  ([what that means](#what-an-unsigned-build-means-for-users)).
- Some exist: the run stops and names the missing ones.

You need a paid Apple Developer Program membership. Only the Account Holder of the team can make
the certificate.

| Secret | What it holds | Where it comes from |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | Your Developer ID Application certificate and its private key, as one line of base64 | developer.apple.com → Certificates, IDs & Profiles → new certificate of type **Developer ID Application**. Install it, then in Keychain Access → login → My Certificates, open the entry, right-click the key and export it as a `.p12` file with a password. Then: `openssl base64 -A -in certificate.p12 -out certificate-base64.txt` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you gave the `.p12` file | You chose it at export |
| `APPLE_SIGNING_IDENTITY` | The name of the certificate, for example `Developer ID Application: Your Name (ABCDE12345)` | `security find-identity -v -p codesigning` |
| `APPLE_ID` | The email of your Apple account | |
| `APPLE_PASSWORD` | An app-specific password, not the password of your Apple account | account.apple.com → Sign-In and Security → App-Specific Passwords |
| `APPLE_TEAM_ID` | Your team ID (10 characters) | developer.apple.com → Account → Membership details |

Add them in the repository's Settings → Secrets and variables → Actions → New repository secret,
or from a terminal:

```bash
gh secret set APPLE_CERTIFICATE < certificate-base64.txt
gh secret set APPLE_CERTIFICATE_PASSWORD      # asks for the value; do the same for the other four
```

Then delete `certificate.p12` and `certificate-base64.txt`. Never commit them.

These are the variables Tauri reads at build time
([Tauri: macOS code signing](https://v2.tauri.app/distribute/sign/macos/)). Nothing in the
repository changes when you turn signing on: add the secrets, run the workflow by hand, download
the `.dmg` files from the run's artifacts with a browser, and test them as you
[test a draft](#test-a-draft). The run also checks that Apple's notarization ticket is on the app
("Check the notarization ticket").

To go back to unsigned builds, delete all six secrets.

## What an unsigned build means for users

Without the secrets the app has an ad-hoc signature: a signature with no certificate behind it.
For a user this means:

- macOS (Gatekeeper) stops the first open and says it could not check the app for malware. The
  user must allow it by hand in System Settings → Privacy & Security → Open Anyway. The steps are
  in the README. Every new version asks again.
- macOS cannot show who made the app, and Apple has not scanned it. The checksum file only proves
  that the file is the one on the Releases page.
- macOS may ask again for permissions (the microphone, folders) after each new version, because
  it does not see two ad-hoc builds as the same app.
- A Mac managed by a company can refuse the app completely.

This is acceptable for early testers who know the project. It is not what people expect from an
app they trust, so turn signing on before you tell a wider audience to download Codly.

## Not done yet

- **Updates from inside the app.** Codly does not look for a new version: users download it again.
  The next step is Tauri's updater plugin. It needs a new dependency (`tauri-plugin-updater`), a
  key pair of its own that signs each update (`pnpm tauri signer generate`, the private key as a
  secret of this workflow), `createUpdaterArtifacts` in `tauri.conf.json`, a `latest.json` file in
  each release, and a "Check for updates" button that never installs while agents run. Do it after
  Developer ID signing is on, so that an update does not bring the Gatekeeper warning back.
