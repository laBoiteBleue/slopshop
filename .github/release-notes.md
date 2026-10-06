An early development build of SlopShop, a non-destructive, GPU-first image editor. See the
[README](https://github.com/laBoiteBleue/slopshop#readme) for what exists today and what does
not exist yet. Expect rough edges, and keep copies of files that matter to you.

## Which file to download

| System                              | File                                           | Tested by hand |
| ----------------------------------- | ---------------------------------------------- | -------------- |
| Windows 10/11 (64-bit)              | `SlopShop_<version>_x64-setup.exe` (or `.msi`) | Yes            |
| macOS, Apple silicon (M1 and later) | `SlopShop_<version>_aarch64.dmg`               | **No**         |
| macOS, Intel                        | `SlopShop_<version>_x64.dmg`                   | **No**         |
| Linux (Debian, Ubuntu)              | `SlopShop_<version>_amd64.deb`                 | **No**         |
| Linux (Fedora, openSUSE)            | `SlopShop-<version>-1.x86_64.rpm`              | **No**         |
| Linux (other distributions)         | `SlopShop_<version>_amd64.AppImage`            | **No**         |

The macOS and Linux builds are produced by continuous integration, where the engine's tests
pass, but nobody has used the application on those systems yet. Reports are very welcome:
[open an issue](https://github.com/laBoiteBleue/slopshop/issues/new/choose), even to say that
it works.

## System requirements

- **Operating system**: Windows 10 or 11 (64-bit); macOS 10.13 or later; Linux x86-64 with
  WebKitGTK 4.1 (Ubuntu 22.04, Debian 12, Fedora 38 or later).
- **Graphics**: a GPU with DirectX 12 (Windows), Metal (macOS) or Vulkan (Linux) drivers;
  integrated graphics are enough.
- **Memory**: 8 GB of RAM; 16 GB or more for images of hundreds of megapixels.
- **Disk**: about 100 MB, plus up to about 1 GB for the optional AI models.
- **AI tools (optional)**: Windows x64, macOS on Apple silicon, Linux; not available on Intel
  Macs.

## The installers are not signed

SlopShop has no code-signing certificate yet, so each system warns before the first launch:

- **Windows**: SmartScreen shows "Windows protected your PC". Click **More info**, then
  **Run anyway**.
- **macOS**: the system refuses to open an app from an unidentified developer. Open
  **System Settings > Privacy & Security**, scroll down, and click **Open Anyway** next to
  SlopShop. If macOS says the app is damaged, run `xattr -cr /Applications/SlopShop.app` in
  Terminal.
- **Linux**: make the AppImage executable (`chmod +x SlopShop_*.AppImage`) before running it.

There is no automatic update yet: new versions are announced on the
[releases page](https://github.com/laBoiteBleue/slopshop/releases).

The optional AI features download their runtime and models on request, after showing their
licenses; nothing is downloaded without your consent.
