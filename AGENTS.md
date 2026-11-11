# YuriEngine 開発環境

## Rust

- Rust の `stable` ツールチェーンを使用します。リポジトリに `rust-toolchain.toml` によるバージョン固定はありません。
- Cargo は Rust ツールチェーンに付属するものを使います。依存関係の解決にはコミット済みの `Cargo.lock` を使い、再現可能なビルドや検証では `--locked` を付けます。
- CI と同じ検証を行う場合は `rustfmt` と `clippy` コンポーネントもインストールしてください。

## Linux のシステム依存

ウィンドウ表示と Linux CI のスモークチェックには以下を使います。

- `libxkbcommon-x11-0`
- `libwayland-dev`
- `libxkbcommon-dev`
- `xvfb`（ヘッドレス環境でウィンドウを開く場合）

ヘッドレスでフレームを描画・キャプチャする場合は Vulkan と Mesa のソフトウェア描画ドライバーも必要です。

- `libvulkan1`
- `mesa-vulkan-drivers`

## ビルドと実行

```sh
cargo build --locked
cargo build --locked --release
```

ウィンドウを使った5フレームのスモークチェック:

```sh
xvfb-run -a cargo run --locked -- --smoke-frames=5
```

Linux のGUIセッションでは `xvfb-run -a` を省略できます。ヘッドレスでPNGを保存する場合:

```sh
cargo run --locked -- --capture-frame=rendered-frame.png
```

## フォーマット・Lint・テスト

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

## Windows

ビルドした実行ファイルの起動には Microsoft Visual C++ 2015–2022 Redistributable（x64）、または互換する Visual C++ ランタイムが必要です。
