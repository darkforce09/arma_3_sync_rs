# packaging

Builds the application into installable packages. CI (`.github/workflows/appimage.yml`) builds
the Linux AppImage on every push to `main` (a workflow artifact) and attaches it to a GitHub
Release for every `v*` tag (a pre-release when the tag has a `-`).

```bash
packaging/linux/build-appimage.sh   # target/appimage/Arma3Sync-<version>-x86_64.AppImage
```

The script needs `cargo`, `curl` and `jq`; it downloads pinned `appimagetool` and AppImage runtime
releases into `target/appimage/` and checks their SHA-256.

The binary links only glibc and loads the display, GPU, tray and file-dialog libraries from the
host, so the AppImage runs on systems whose glibc is at least the build machine's. CI builds on
Ubuntu 22.04 (glibc 2.35); a build on a newer system runs only on systems as new.

The self-updater does not update an AppImage: download the new AppImage instead.
