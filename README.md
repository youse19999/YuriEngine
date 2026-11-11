# YuriEngine SwapChain (Render/SwapChain)

`winit` 0.30 + `wgpu` application that clears the window to opaque pink and presents frames.

## Local build

```sh
cargo build
cargo build --release
```

## Scene and game objects

Imported FBX files are stored as model assets in a `Scene`. Each spawned `GameObject` has a
transform and components such as `MeshRenderer` and `Animator`; parent transforms are combined
when drawing. The same model asset can be referenced by more than one object.

Pass `--model` more than once to create one game object per FBX. An optional animation FBX is
retargeted onto each model:

```sh
cargo run -- --model=assets/character.fbx --model=assets/prop.fbx
cargo run -- --model=assets/character.fbx --animation=assets/walk.fbx
```

Scene files use BSON. Starting with `--save-scene` saves the current scene when the application
closes; `--scene` loads a saved scene and its FBX paths:

```sh
cargo run -- --model=assets/character.fbx --save-scene=assets/demo.bson
cargo run -- --scene=assets/demo.bson --save-scene=assets/demo.bson
```

The BSON document stores object names, parent indices, local transforms, animator playback state,
and each `MeshRenderer` FBX path as a string. Model data and textures are re-imported from those
paths when the scene is loaded.

The application opens an 800 × 600 window and exits when the window is closed. For an automated
window/present smoke check, run `cargo run -- --smoke-frames=5`; it exits after five frames have
been presented. Linux CI runs this under Xvfb, while Windows and macOS CI use their hosted GUI
session.

To render the triangle and ImGui sample into a headless GPU texture and save it as a PNG, run:

```sh
cargo run -- --capture-frame=rendered-frame.png
```

This path uses a fallback wgpu adapter and reads the rendered texture back to the CPU, so it does not require a window system. The Capture Rendered Frame GitHub Actions workflow uploads the PNG as the `yuriengine-rendered-frame` artifact.

## Windows runtime

The current MSVC artifacts import `VCRUNTIME140.dll` and Universal CRT API sets. A Windows computer needs the Microsoft Visual C++ 2015–2022 Redistributable (x64) or an equivalent compatible runtime installation to launch the executable.

## GitHub Actions

GitHub Actions runs format, Clippy, tests, build, and the finite-frame present smoke check on
Windows, Linux, and macOS. A successful smoke check confirms that the program created a window
and presented frames in that runner; it is separate from a visual pixel inspection.
