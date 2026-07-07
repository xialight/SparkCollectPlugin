# SparkCollectPlugin (v2)

A [Hachimi](https://github.com/kairusds/Hachimi-Edge) plugin for Umamusume
Pretty Derby that automatically submits your career results — and every spark
list you see while rerolling — to the umaspark worker API using your personal
token.

## How it works

The plugin hooks `Gallop.HttpHelper.DecompressResponse` to intercept game API
responses (already decrypted and decompressed by the time this method
returns) in memory, and watches for three packet kinds:

- `single_mode_finish_common` — a finished career. Submitted the same way as
  the original plugin: one "run" with the character's final committed sparks.
- `single_mode_factor_select_common` — the original, pre-reroll spark list.
- `single_mode_factor_lottery_common` — the result of a single reroll action.

For the latter two, the plugin reads **every** candidate list in
`factor_select_info_array` (not just the newest one) and submits each as its
own separate run, tagged with its `lottery_id`. Rerolling five times produces
five distinct submissions, each independently identifiable later.

Every spark — whether from a finished career or a reroll candidate — is
decoded from the game's packed `factor_id` integer using the same bucketing
rules: 100–599 stat, 1000–3999 aptitude, 1,000,000–1,999,999 race (white)
spark, 2,000,000–2,999,999 skill (white) spark, 3,000,000–3,999,999 scenario
(white) spark, ≥10,000,000 unique/character spark.

## Build

The game is 64-bit (Unity 2022.3.62f2) — `.cargo/config.toml` sets
`x86_64-pc-windows-msvc` as the default build target and `il2cpp` (the
2022-generation IL2CPP bindings) as the default feature, so a plain build
just works:

```powershell
$env:SPARK_WORKER_URL = "https://subdomain.domain.com"
cargo build --release
```

Or edit `.cargo/config.toml` to set `SPARK_WORKER_URL` permanently and just
run `cargo build --release`.

The output DLL is at `target/x86_64-pc-windows-msvc/release/spark_collect_plugin.dll`.

> **Note:** the game was previously 32-bit on Unity 2020.3.24f1 — if it ever
> reverts or a differently-built client needs the old target, use
> `cargo build --release --target i686-pc-windows-msvc --no-default-features --features il2cpp_2020`
> (needs `rustup target add i686-pc-windows-msvc` first). Check the actual
> client before assuming either — see the "il2cpp bindings" note below.

Run `cargo test` (no target flag needed — the msgpack parsing/routing logic
has no IL2CPP dependency) to run the unit tests covering spark decoding and
packet routing.

## Repository structure

```
src/
├── lib.rs          Plugin entry point, hook setup, packet routing, dedup
├── extract.rs       msgpack parsing — finish summary AND spark-reroll lists
└── worker.rs         HTTP POST to the worker API (finish runs + reroll runs)

hachimi_plugin_sdk/         Vendored Hachimi plugin SDK (root package)
hachimi_plugin_macros/      Proc macro crate (part of the SDK)
hachimi_il2cpp/             IL2CPP bindings for modern Unity (currently used —
                             the game is on Unity 2022.3.62f2, 64-bit)
hachimi_il2cpp_2020/        IL2CPP bindings for Unity 2020 (32-bit) — no
                             longer matches the live client, kept for reference

.cargo/config.toml          Default SPARK_WORKER_URL (edit before building)
```

## Configuration

After the first launch with the plugin installed, a
`SparkCollectPlugin/config.properties` file is created in your game root:

```properties
# Your personal token from umaspark.rappy.dev/settings
token=

# Minimum seconds between submissions (prevents double-uploads on quick retries).
# Only applies to career-finish submissions — reroll submissions are deduped
# by lottery_id instead, since rapid repeated rerolls are distinct real events.
cooldown_seconds=30

# Set to true to save raw response JSON to SparkCollectPlugin/logs/ for debugging.
# Files are named finish_<ts>.json / factor_select_<ts>.json / factor_lottery_<ts>.json.
debug_raw=false

# Set to false to disable submitting spark-reroll data (factor_select/factor_lottery).
# Useful to turn off during initial rollout if the worker isn't ready to accept these yet.
submit_factor_lists=true
```

## Known worker-side limitations (as of this writing)

The deployed worker (`spark-tracker-worker/src/plugin-run.js`) was built
before this feature and currently:

- Hard-requires `character_id` as a positive integer on every submission.
  Reroll-list submissions send `character_id: null` — the worker will reject
  these until it's updated.
- Hard-requires `stat_spark` and `aptitude_spark` to be present. Reroll
  entries often only populate some spark categories, and the worker will
  reject entries missing either.
- Throttles to one submission per Discord user per 5 minutes, which will cap
  rapid-fire reroll submissions well below what the plugin actually sends.

These are expected today and are follow-up work for the worker, not bugs in
this plugin. Every submission still includes a `"kind"` field (`"finish"`,
`"factor_select"`, or `"factor_lottery"`) so the worker can eventually branch
its validation per kind.
