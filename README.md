# SparkCollectPlugin

A [Hachimi](https://github.com/kairusds/Hachimi-Edge) plugin for Umamusume
Pretty Derby that automatically submits your career results to
[umaspark.rappy.dev](https://umaspark.rappy.dev) after each run.

## How it works

The plugin hooks `HttpHelper.DecompressResponse` to intercept game API
responses in memory, detects career-finish packets, extracts the spark data,
and POSTs it directly to the umaspark worker API using your personal token.

## Build

Requires Rust with the `i686-pc-windows-msvc` target (the game is 32-bit):

```powershell
rustup target add i686-pc-windows-msvc
```

Set the worker URL and build:

```powershell
$env:SPARK_WORKER_URL = "https://subdomain.domain.com"
cargo build --release --target i686-pc-windows-msvc
```

Or edit `.cargo/config.toml` to set `SPARK_WORKER_URL` permanently and just
run `cargo build --release`.

The output DLL is at `target/i686-pc-windows-msvc/release/spark_collect_plugin.dll`.

## Repository structure

```
src/
├── lib.rs          Plugin entry point, hook setup, response processing
├── extract.rs      msgpack parsing — finds and extracts finish summary
└── worker.rs       HTTP POST to the worker API

hachimi_plugin_sdk/         Vendored Hachimi plugin SDK (root package)
hachimi_plugin_macros/      Proc macro crate (part of the SDK)
hachimi_il2cpp/             IL2CPP bindings for modern Unity
hachimi_il2cpp_2020/        IL2CPP bindings for Unity 2020

.cargo/config.toml          Default SPARK_WORKER_URL (edit before building)
```

## Configuration

After the first launch with the plugin installed, a
`SparkCollectPlugin/config.properties` file is created in your game root:

```properties
# Your personal token from umaspark.rappy.dev/settings
token=

# Minimum seconds between submissions (prevents double-uploads on quick retries)
cooldown_seconds=30

# Set to true to save raw response JSON to SparkCollectPlugin/logs/ for debugging
debug_raw=false
```