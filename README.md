# YuriEngine Inspection/001

Minimal `winit` window application used to validate a Windows x64 Rust build.

## Local build

```sh
cargo build
cargo build --release
```

The application opens an 800 × 600 window and exits when the window is closed.

## GitHub Actions

The Windows x64 workflow builds Debug and Release for `x86_64-pc-windows-msvc` and uploads one executable artifact per profile. CI build success confirms compilation and packaging; it does not replace a Windows GUI launch check. The Windows runner's GUI launch behavior is not part of this workflow.
