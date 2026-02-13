# Arch Linux Packaging

This repository includes two Arch package recipes:

- `packaging/arch/PKGBUILD`: release package from a tagged source archive (`vX.Y.Z`).
- `packaging/arch/PKGBUILD-git`: VCS package from the latest git revision.
  - Set `HYPRFINITY_GIT_SOURCE=local` to build from the local checkout (offline-friendly).

Lockfile behavior:

- If `Cargo.lock` exists in source, packaging uses `cargo ... --locked`.
- If `Cargo.lock` is absent, packaging falls back to unlocked Cargo build/test.

## Build a release package

From the repository root:

```bash
makepkg -p packaging/arch/PKGBUILD -si
```

Notes:

- The release PKGBUILD expects tags in the form `v0.1.0`.
- Before publishing to AUR, replace `sha256sums=('SKIP')` with a real checksum:

```bash
updpkgsums packaging/arch/PKGBUILD
```

## Build a git package

```bash
makepkg -p packaging/arch/PKGBUILD-git -si
```

Build from local repository checkout instead of GitHub:

```bash
cd packaging/arch
HYPRFINITY_GIT_SOURCE=local makepkg -p PKGBUILD-git -si
```

The `-git` package:

- Tracks the latest commit from the repository.
- Provides `hyprfinity` and conflicts with the release package.

## Makefile shortcuts

From repository root:

```bash
make arch-build-release
make arch-build-git
make arch-build-git-local
```
