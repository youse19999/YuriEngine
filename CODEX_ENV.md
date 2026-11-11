# Codex クラウド環境セットアップ

このガイドは、Linux ベースの Codex クラウド環境で YuriEngine をビルド・検証する手順です。リポジトリは Rust の `stable` を使用します。Rust のバージョンは固定されていないため、`rust-toolchain.toml` が追加された場合はその指定に従ってください。

## 1. 作業ディレクトリを確認

Codex がこのリポジトリをチェックアウトしたディレクトリで作業します。

```sh
pwd
git status --short
```

## 2. OS 依存パッケージをインストール

Debian / Ubuntu 系の環境では、ウィンドウ関連の依存パッケージを導入します。Codex 環境で `sudo` が使えない場合は、環境イメージの作成時に同じパッケージを追加してください。

```sh
sudo apt-get update
sudo apt-get install --yes \
  libxkbcommon-x11-0 \
  libwayland-dev \
  libxkbcommon-dev \
  xvfb
```

ヘッドレスでGPU描画を行い、PNGをキャプチャする場合は Vulkan と Mesa のソフトウェアドライバーも導入します。

```sh
sudo apt-get install --yes libvulkan1 mesa-vulkan-drivers
```

## 3. Rust stable と検証コンポーネントを用意

`rustc`、`cargo`、`rustfmt`、`clippy` が PATH 上で利用できることを確認します。既に用意されていればインストールを省略できます。

```sh
rustup toolchain install stable --component rustfmt --component clippy
rustup default stable
rustc --version
cargo --version
rustup component list --installed
```

## 4. 依存関係の取得とビルド

`Cargo.lock` に記録された依存関係を使ってビルドします。

```sh
cargo fetch --locked
cargo build --locked
```

最適化ビルドが必要な場合:

```sh
cargo build --locked --release
```

## 5. フォーマット、Lint、テスト

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

## 6. 実行確認

Codex クラウド環境にディスプレイがない場合、Xvfb 上で有限フレームのウィンドウ表示チェックを行います。

```sh
xvfb-run -a cargo run --locked -- --smoke-frames=5
```

Vulkan ソフトウェア描画を利用してPNGを生成する場合:

```sh
cargo run --locked -- --capture-frame=rendered-frame.png
```

生成した `rendered-frame.png` は作業ファイルです。成果物としてユーザーに渡す場合は、プロジェクトで指定された成果物ディレクトリに保存してください。

## 補足

- CI の Linux 検証は `libxkbcommon-x11-0`、`libwayland-dev`、`libxkbcommon-dev`、`xvfb` を使い、`xvfb-run` で5フレームのスモークチェックを実行します。
- CI のキャプチャジョブは `libvulkan1` と `mesa-vulkan-drivers` を追加し、`--capture-frame` を実行します。
- Windows のビルド済み実行ファイルを起動する場合は Microsoft Visual C++ 2015–2022 Redistributable（x64）または互換ランタイムが必要です。これは Linux クラウド環境のセットアップには不要です。
