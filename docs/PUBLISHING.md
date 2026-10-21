# Publishing this prepared repository

The repository is prepared locally. No GitHub repository, remote, commit, tag or release has been created by the preparation step.

Publish the **source repository**, including `src/`, `Cargo.toml`, `Cargo.lock`, `BUILD.ps1`, the documentation, licenses and hidden `.github` and `.cargo` folders. The Windows executable ZIP belongs in GitHub Releases, not in the source tree.

If starting from `AulaStudio-0.1.0-source.zip`, extract it, open PowerShell in its `AulaStudio` folder, and run `git init -b main` before the steps below. The source archive intentionally contains no `.git` history, private toolchain, build output or personal settings. If using the already prepared local repository, Git is initialized and this extra step is unnecessary.

## First publication

1. Review `git status --short` and the source. `.gitignore` excludes development tools, local verification evidence, profiles, checkpoints and generated releases.
2. Create an empty public repository in your GitHub account. Do not generate another README, license or `.gitignore`.
3. Add the files, review the staged list and create the initial commit:

   ```powershell
   git add .cargo .github .gitattributes .gitignore BUILD.ps1 CONTRIBUTING.md Cargo.lock Cargo.toml LICENSE README.md SECURITY.md THIRD_PARTY_NOTICES.md rust-toolchain.toml assets docs licenses src tools
   git diff --cached --check
   git diff --cached --stat
   git commit -m "Prepare Aula Studio USB release"
   ```

4. Copy the remote URL from your empty GitHub repository, then run `git remote add origin <your-repository-url>` and `git push -u origin main`, replacing the placeholder with your URL.
5. Confirm the Windows checks and dependency-audit workflows complete successfully. Their configuration is checked locally, but hosted runs need the published repository.
6. Enable private vulnerability reporting in the repository's security settings. Add a description and the topics `rust`, `windows`, `keyboard`, `aula-f75` and `rgb`.
7. Add the final repository URL to the package metadata in `Cargo.toml` when it is known.

## Downloadable release

After tests and hosted CI pass:

```powershell
.\BUILD.ps1 fmt
.\BUILD.ps1 test
.\BUILD.ps1 clippy
.\BUILD.ps1 package
Get-FileHash .\dist\releases\AulaStudio-0.1.0-windows-x86_64.zip -Algorithm SHA256
```

Create a `v0.1.0` tag and GitHub release from the verified commit. Attach the ZIP and its `.sha256` file from `dist/releases`. Preserve the full archive, including recovery utility and license notices. Do not upload `verification`, `.tools`, OEM binaries or personal checkpoints.

Describe the release as Windows x64, USB-only, unsigned, and tested with the included ISO/French F75 layout. Link `docs/RELIABILITY.md` and record checks for the exact release commit. Do not claim universal firmware or Windows-version compatibility.

For later releases, update the Cargo package version and lockfile, regenerate affected license notices after dependency/toolchain changes, rerun checks, inspect the package contents, and use a new tag.
