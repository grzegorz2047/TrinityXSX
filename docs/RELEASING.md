# Releasing TrinityXSX

TrinityXSX uses a **draft-first release flow** (adapted from `TrinityX360`): releases are built by GitHub Actions from a `release/X.Y.Z` branch into a **Draft GitHub Release**. No `vX.Y.Z` tag is created until the draft's built assets have been verified on a PS5 and published by hand.

## Release steps

1. **Create a release branch from `main`:**
   ```bash
   git checkout main && git pull
   git checkout -b release/0.2.21
   ```
2. **Bump version metadata in one commit:**
   - `library/library.rs`: update `TRINITYXSX_VERSION` to `"v0.2.21"`.
   - `sce_sys/param.json`: update `contentVersion` to `"01.002.021"` (`NN.NNN.NNN`, matching `tools/release_guard.py content-version --version 0.2.21`).
   - `docs/release-notes/v0.2.21.md`: write the release notes for this version. Use explicit verification tags where helpful:
     - `[console]` — confirmed on a PS5.
     - `[console, partly]` — partially confirmed on a PS5 (explain what remains unconfirmed).
     - `[host tests]` — verified by host/CI unit tests only.
   - Check everything locally before pushing:
     ```bash
     python3 -I tools/release_guard.py check-repo --version 0.2.21 --root .
     python3 -m unittest tools.test_release_guard -v
     ```
3. **Push `release/0.2.21` to GitHub:**
   - `.github/workflows/release.yml` validates metadata and release notes (`prepare`), builds all assets (`xcl`, `vkd3d-proton`, `innoextract`, `txin-sender`, `standalone-ffpkg`), verifies `PPSA83601.zip`'s `sce_sys/` against the repository, generates `SHA256SUMS.txt` (`package`), re-verifies every file's SHA-256 (`draft`), and creates or updates the **Draft Release `v0.2.21`** targeting that exact commit.
   - Pushing a fix commit to `release/0.2.21` safely updates the existing draft (`--clobber`). Published releases are never modified (`tools/release_guard.py decide` exits with code 2).
4. **Verify on PS5:**
   - Download `PPSA83601-v0.2.21.ffpkg.zip` (and `txin-sender-v0.2.21.exe`) from the draft release, verify SHA-256 against `SHA256SUMS.txt`, deploy to PS5 with backup `.before-*`, and test.
5. **Publish by hand:**
   - Once verified, publish the draft release on GitHub (which creates tag `v0.2.21` on the exact tested commit).
   - Merge `release/0.2.21` back into `main` (via PR or fast-forward) so `main` keeps the updated `TRINITYXSX_VERSION`, `sce_sys/param.json`, and `docs/release-notes/v0.2.21.md`.
