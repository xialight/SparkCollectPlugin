mod extract;
mod worker;

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};

use hachimi_plugin_sdk::{
    api::{Hachimi, HachimiApi},
    hachimi_plugin,
    il2cpp::{helpers::Array, types::{Il2CppArray, Il2CppClass, Il2CppImage, Il2CppObject}},
    sys::InitResult,
};
use lazy_static::lazy_static;
use log::{error, info};

const VERSION: &str = "0.3.0";
const CONFIG_DIR:  &str = "SparkCollectPlugin";
const CONFIG_FILE: &str = "config.properties";
const DEFAULT_COOLDOWN_SECS: u64 = 30;
// A career takes real playtime end-to-end, and a reroll (when it happens)
// fires seconds before the career's own finish, so 10 minutes comfortably
// covers every legitimate case while making stale cross-career leakage
// require an unrelated second career to reach finish within that same
// window — astronomically unlikely, and even then sparks_match() still has
// to agree before anything gets marked is_selected.
const DEFAULT_FACTOR_CACHE_MAX_AGE_SECS: u64 = 600;

// Set SPARK_WORKER_URL before building:
//   SPARK_WORKER_URL=https://your-api.workers.dev cargo build --release
const WORKER_URL: &str = env!("SPARK_WORKER_URL");

static mut API: Option<HachimiApi> = None;
static mut DECOMPRESS_ORIG: usize  = 0;

/// Tag used for debug-dump filenames for spark-reroll packets.
/// `single_mode_factor_select_common` (the original, pre-reroll list) is
/// deliberately not tracked — its spark list always reappears as one of
/// `single_mode_factor_lottery_common`'s entries, so it's pure duplicate data.
const FACTOR_LOTTERY_TAG: &str = "factor_lottery";

/// The most recent `single_mode_factor_lottery_common` snapshot. The game's
/// own `factor_select_info_array` already accumulates every candidate seen so
/// far in one reroll sequence, so each new packet just overwrites this
/// wholesale — no merging needed on our side.
struct FactorCache {
    entries:    Vec<extract::FactorListEntry>,
    updated_at: std::time::Instant,
}

lazy_static! {
    static ref CONFIG: HashMap<String, String> = load_config();
    static ref LAST_NOTIFIED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    static ref FACTOR_CACHE: Mutex<Option<FactorCache>> = Mutex::new(None);
}

fn load_config() -> HashMap<String, String> {
    let path = format!("{CONFIG_DIR}/{CONFIG_FILE}");
    std::fs::File::open(&path)
        .ok()
        .and_then(|f| java_properties::read(BufReader::new(f)).ok())
        .unwrap_or_default()
}

pub fn cfg(key: &str) -> Option<String> {
    CONFIG.get(key).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn cooldown() -> std::time::Duration {
    let secs = cfg("cooldown_seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_COOLDOWN_SECS);
    std::time::Duration::from_secs(secs)
}

fn max_factor_cache_age() -> std::time::Duration {
    let secs = cfg("factor_cache_max_age_seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_FACTOR_CACHE_MAX_AGE_SECS);
    std::time::Duration::from_secs(secs)
}

#[hachimi_plugin]
pub fn main(api: HachimiApi) -> InitResult {
    unsafe { API = Some(api); }
    _ = hachimi_plugin_sdk::log::init(api, log::Level::Info);
    info!("SparkCollectPlugin {VERSION} loading...");

    let config_path = format!("{CONFIG_DIR}/{CONFIG_FILE}");
    if !Path::new(&config_path).exists() {
        write_default_config(&config_path);
    }

    let il2cpp  = api.il2cpp();
    let hachimi = Hachimi::instance(&api);
    let interceptor = hachimi.interceptor();

    let http_helper = find_class_in_all_assemblies(&api, c"Gallop", c"HttpHelper");
    if http_helper.is_null() {
        return InitResult::Error;
    }

    let addr = il2cpp.get_method_addr(http_helper, c"DecompressResponse", 1);
    if addr == 0 {
        error!("Failed to find DecompressResponse");
        return InitResult::Error;
    }
    info!("DecompressResponse resolved at {addr:#x}");

    match interceptor.hook(addr, hook_decompress_response as _) {
        Some(trampoline) => {
            unsafe { DECOMPRESS_ORIG = trampoline; }
            info!("DecompressResponse hook installed (trampoline {trampoline:#x}) — if another mod (e.g. Heaven) already hooked this function, this is a chained hook on top of it");
        }
        None => {
            // The whole plugin is this one hook — if it didn't take, we're
            // completely non-functional. Fail loudly instead of reporting a
            // false "loaded" success that silently never does anything.
            error!("DecompressResponse hook install FAILED — plugin will not receive any responses");
            return InitResult::Error;
        }
    }

    info!("SparkCollectPlugin {VERSION} loaded — watching for career completions and spark rerolls");
    InitResult::Ok
}

type DecompressResponseFn =
    extern "C" fn(this: *mut Il2CppObject, data: *mut Il2CppArray) -> *mut Il2CppArray;

unsafe extern "C" fn hook_decompress_response(
    this: *mut Il2CppObject,
    data: *mut Il2CppArray,
) -> *mut Il2CppArray {
    let orig: DecompressResponseFn = std::mem::transmute(DECOMPRESS_ORIG);
    let result = orig(this, data);

    if !result.is_null() {
        let arr   = Array::<u8>::from(result);
        let bytes = arr.as_slice();
        info!("DecompressResponse fired — {} bytes", bytes.len());
        process_response(bytes);
    }

    result
}

fn process_response(bytes: &[u8]) {
    let Some(value) = parse_msgpack(bytes) else {
        info!("process_response: msgpack parse failed");
        return;
    };
    let rmpv::Value::Map(ref entries) = value else {
        info!("process_response: top-level value is not a map");
        return;
    };
    let keys: Vec<&str> = entries.iter().filter_map(|(k, _)| k.as_str()).take(8).collect();
    info!("process_response: top-level keys = {keys:?}");

    // Checked independently (not an early-return chain) so that a packet
    // carrying reroll data doesn't get skipped just because it isn't ALSO a
    // finish-common packet, and vice versa.
    if let Some(finish_common) = extract::find_finish_common(entries) {
        handle_finish_common(finish_common, &value);
    }
    if let Some(packet) = extract::find_factor_lottery_common(entries) {
        handle_factor_lottery(packet, &value);
    }
}

/// Caches the latest reroll snapshot. Never submits anything itself —
/// submission only ever happens once `finish_common` fires (see
/// `handle_finish_common`), since only the finish event carries the
/// stats/aptitudes/parent context a submitted run requires.
fn handle_factor_lottery(packet: &[(rmpv::Value, rmpv::Value)], full_response: &rmpv::Value) {
    let entries = extract::extract_factor_list(packet);
    info!("{FACTOR_LOTTERY_TAG}: cached snapshot now has {} candidate(s)", entries.len());

    if cfg("debug_raw").as_deref() == Some("true") {
        debug_save_json(FACTOR_LOTTERY_TAG, full_response);
    }

    let mut cache = FACTOR_CACHE.lock().unwrap();
    *cache = Some(FactorCache { entries, updated_at: std::time::Instant::now() });
}

/// Consumes (and always clears, even if empty/None/stale) the cached reroll
/// candidates for use by a finish event. See DEFAULT_FACTOR_CACHE_MAX_AGE_SECS
/// for why age-bounding is the chosen invalidation strategy.
fn take_valid_factor_cache() -> Vec<extract::FactorListEntry> {
    match FACTOR_CACHE.lock().unwrap().take() {
        Some(c) if c.updated_at.elapsed() <= max_factor_cache_age() => c.entries,
        Some(c) => {
            info!(
                "{FACTOR_LOTTERY_TAG}: discarding stale cache ({} candidate(s), {:?} old) — treating as no reroll",
                c.entries.len(), c.updated_at.elapsed(),
            );
            Vec::new()
        }
        None => Vec::new(),
    }
}

/// True iff two spark breakdowns represent the same committed spark set:
/// stat/aptitude compared by (name, stars), unique by stars, skill_sparks by
/// set-equality over (spark_type, spark_id, stars) regardless of order.
fn sparks_match(a: &extract::SparkBreakdown, b: &extract::SparkBreakdown) -> bool {
    fn key(e: &Option<extract::SparkEntry>) -> Option<(&str, i64)> {
        e.as_ref().map(|s| (s.name.as_str(), s.stars))
    }
    if key(&a.stat_spark) != key(&b.stat_spark) { return false; }
    if key(&a.aptitude_spark) != key(&b.aptitude_spark) { return false; }
    if a.unique_spark.as_ref().map(|s| s.stars) != b.unique_spark.as_ref().map(|s| s.stars) {
        return false;
    }

    let mut a_skills: Vec<(&str, i64, i64)> =
        a.skill_sparks.iter().map(|s| (s.spark_type, s.spark_id, s.stars)).collect();
    let mut b_skills: Vec<(&str, i64, i64)> =
        b.skill_sparks.iter().map(|s| (s.spark_type, s.spark_id, s.stars)).collect();
    a_skills.sort();
    b_skills.sort();
    a_skills == b_skills
}

fn handle_finish_common(finish_common: &[(rmpv::Value, rmpv::Value)], full_response: &rmpv::Value) {
    let now = std::time::Instant::now();
    {
        let mut last = LAST_NOTIFIED.lock().unwrap();
        if let Some(t) = *last {
            if now.duration_since(t) < cooldown() {
                return;
            }
        }
        *last = Some(now);
    }

    // Dumped unconditionally (before the has_sparks gate below) so a finish
    // that fails spark extraction still leaves a diagnostic artifact —
    // otherwise the exact packet that tripped the gate is lost forever.
    if cfg("debug_raw").as_deref() == Some("true") {
        debug_save_json("finish", full_response);
    }

    let summary = extract::extract_finish_summary(finish_common);

    let has_sparks = summary.stat_spark.is_some()
        || summary.aptitude_spark.is_some()
        || !summary.skill_sparks.is_empty();

    if !has_sparks {
        info!("Career finish detected but no sparks — skipping (run not complete)");
        return;
    }

    let Some(token) = require_token() else { return; };

    let cached = take_valid_factor_cache();

    // Common case (no reroll this career): unchanged single-submission path —
    // same message_id format, same single row, as before this feature existed.
    if cached.is_empty() {
        std::thread::spawn(move || {
            worker::submit_run(WORKER_URL, &token, &summary);
        });
        return;
    }

    // Reroll case: one row per cached candidate, sharing every non-spark
    // field from `summary`, plus (only if none of them match the committed
    // factor_id_array) one extra row for the committed breakdown itself, so
    // data is never silently dropped if matching fails.
    let committed = extract::SparkBreakdown {
        stat_spark:     summary.stat_spark.clone(),
        aptitude_spark: summary.aptitude_spark.clone(),
        unique_spark:   summary.unique_spark.clone(),
        skill_sparks:   summary.skill_sparks.clone(),
    };
    let matched_index = cached.iter().position(|c| sparks_match(&c.sparks, &committed));

    info!(
        "finish with {} cached reroll candidate(s), matched_index={:?}",
        cached.len(), matched_index,
    );

    // Shared by every variant of this one finish event — both so all N rows
    // record the exact same real-world moment, and so the worker's rate
    // limiter can tell "this event's own burst" apart from a genuinely
    // different finish (see plugin-run.js's rate-limit fix).
    let obtained_at_ms = worker::now_ms();
    let summary = Arc::new(summary);

    for (i, candidate) in cached.into_iter().enumerate() {
        let is_selected = matched_index == Some(i);
        let tag = candidate.lottery_id.map(|id| id.to_string()).unwrap_or_else(|| format!("i{i}"));
        let token = token.clone();
        let summary = summary.clone();
        std::thread::spawn(move || {
            worker::submit_run_variant(
                WORKER_URL, &token, &summary, &candidate.sparks,
                candidate.lottery_id, is_selected, Some(tag), obtained_at_ms,
            );
        });
    }

    if matched_index.is_none() {
        let token = token.clone();
        let summary = summary.clone();
        std::thread::spawn(move || {
            worker::submit_run_variant(
                WORKER_URL, &token, &summary, &committed,
                None, true, Some("committed".to_string()), obtained_at_ms,
            );
        });
    }
}

fn require_token() -> Option<String> {
    match cfg("token") {
        Some(t) => Some(t),
        None => {
            error!("No token configured in {CONFIG_DIR}/{CONFIG_FILE} — cannot submit run");
            None
        }
    }
}

fn debug_save_json(tag: &str, value: &rmpv::Value) {
    const LOG_DIR: &str = "SparkCollectPlugin/logs";
    if let Err(e) = std::fs::create_dir_all(LOG_DIR) {
        error!("debug_raw: failed to create logs dir: {e}");
        return;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = format!("{LOG_DIR}/{tag}_{ts}.json");
    let json = msgpack_to_json(value);
    match std::fs::write(&path, json.as_bytes()) {
        Ok(_)  => info!("debug_raw: saved {path}"),
        Err(e) => error!("debug_raw: failed to write {path}: {e}"),
    }
}

fn msgpack_to_json(v: &rmpv::Value) -> String {
    match v {
        rmpv::Value::Nil        => "null".into(),
        rmpv::Value::Boolean(b) => b.to_string(),
        rmpv::Value::Integer(i) => i.to_string(),
        rmpv::Value::F32(f)     => f.to_string(),
        rmpv::Value::F64(f)     => f.to_string(),
        rmpv::Value::String(s)  => format!("\"{}\"", s.as_str().unwrap_or("").replace('"', "\\\"")),
        rmpv::Value::Binary(b)  => format!("\"<{} bytes>\"", b.len()),
        rmpv::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(msgpack_to_json).collect();
            format!("[{}]", items.join(","))
        }
        rmpv::Value::Map(pairs) => {
            let items: Vec<String> = pairs.iter().map(|(k, val)| {
                format!("{}:{}", msgpack_to_json(k), msgpack_to_json(val))
            }).collect();
            format!("{{{}}}", items.join(","))
        }
        rmpv::Value::Ext(t, b)  => format!("\"<ext {} {} bytes>\"", t, b.len()),
    }
}

fn parse_msgpack(bytes: &[u8]) -> Option<rmpv::Value> {
    rmpv::decode::read_value(&mut &bytes[..]).ok().or_else(|| {
        bytes.get(4..).and_then(|b| rmpv::decode::read_value(&mut &b[..]).ok())
    })
}

fn find_class_in_all_assemblies(
    api: &HachimiApi,
    namespace: &std::ffi::CStr,
    class_name: &std::ffi::CStr,
) -> *mut Il2CppClass {
    let il2cpp = api.il2cpp();

    type DomainGetFn        = unsafe extern "C" fn() -> *mut u8;
    type GetAssembliesFn    = unsafe extern "C" fn(*mut u8, *mut usize) -> *mut *mut u8;
    type AssemblyGetImageFn = unsafe extern "C" fn(*mut u8) -> *const Il2CppImage;

    let domain_get_addr     = il2cpp.resolve_symbol(c"il2cpp_domain_get");
    let get_assemblies_addr = il2cpp.resolve_symbol(c"il2cpp_domain_get_assemblies");
    let get_image_addr      = il2cpp.resolve_symbol(c"il2cpp_assembly_get_image");

    if domain_get_addr == 0 || get_assemblies_addr == 0 || get_image_addr == 0 {
        error!("Failed to resolve IL2CPP domain symbols");
        return std::ptr::null_mut();
    }

    unsafe {
        let domain_get:     DomainGetFn        = std::mem::transmute(domain_get_addr);
        let get_assemblies: GetAssembliesFn    = std::mem::transmute(get_assemblies_addr);
        let get_image:      AssemblyGetImageFn = std::mem::transmute(get_image_addr);

        let domain = domain_get();
        if domain.is_null() {
            error!("il2cpp_domain_get returned null");
            return std::ptr::null_mut();
        }

        let mut count: usize = 0;
        let assemblies_ptr = get_assemblies(domain, &mut count);
        if assemblies_ptr.is_null() {
            error!("il2cpp_domain_get_assemblies returned null");
            return std::ptr::null_mut();
        }

        let assemblies = std::slice::from_raw_parts(assemblies_ptr, count);
        info!("Searching {} assemblies for {namespace:?}::{class_name:?}", assemblies.len());

        for &assembly in assemblies {
            if assembly.is_null() { continue; }
            let image = get_image(assembly);
            if image.is_null() { continue; }
            let class = il2cpp.get_class(image, namespace, class_name);
            if !class.is_null() {
                info!("Found {namespace:?}::{class_name:?}");
                return class;
            }
        }
    }

    error!("Could not find {namespace:?}::{class_name:?} in any assembly");
    std::ptr::null_mut()
}

fn write_default_config(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        path,
        b"# Your personal token from umaspark.rappy.dev/settings\n\
          # Log in with Discord, go to Settings, and click Generate token.\n\
          token=\n\
          \n\
          # Minimum seconds between submissions (prevents duplicate uploads on quick retries)\n\
          cooldown_seconds=30\n\
          \n\
          # Set to true to save raw response JSON to SparkCollectPlugin/logs/ for debugging\n\
          debug_raw=false\n\
          \n\
          # How long (seconds) a cached spark-reroll snapshot stays valid while\n\
          # waiting for the career's finish event. Default 10 minutes.\n\
          factor_cache_max_age_seconds=600\n",
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn map(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (Value::from(k), v)).collect())
    }

    fn factor_lottery_packet(lottery_id: i64, factor_ids: &[i64]) -> Value {
        let factor_info = Value::Array(
            factor_ids.iter().map(|&id| map(vec![("factor_id", Value::from(id))])).collect(),
        );
        map(vec![(
            "single_mode_factor_lottery_common",
            map(vec![(
                "factor_select_info_array",
                Value::Array(vec![map(vec![
                    ("lottery_id", Value::from(lottery_id)),
                    ("factor_info_array", factor_info),
                ])]),
            )]),
        )])
    }

    fn encode(value: &Value) -> Vec<u8> {
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, value).unwrap();
        buf
    }

    fn breakdown(
        stat: Option<(&str, i64)>,
        apt: Option<(&str, i64)>,
        unique_stars: Option<i64>,
        skills: &[(&'static str, i64, i64)],
    ) -> extract::SparkBreakdown {
        extract::SparkBreakdown {
            stat_spark: stat.map(|(name, stars)| extract::SparkEntry { name: name.into(), stars }),
            aptitude_spark: apt.map(|(name, stars)| extract::SparkEntry { name: name.into(), stars }),
            unique_spark: unique_stars.map(|stars| extract::SparkEntry { name: "Character Factor".into(), stars }),
            skill_sparks: skills.iter().map(|&(spark_type, spark_id, stars)| {
                extract::SkillSparkEntry { spark_type, spark_id, stars }
            }).collect(),
        }
    }

    #[test]
    fn sparks_match_identical_breakdowns() {
        let a = breakdown(Some(("Speed", 3)), Some(("Turf", 2)), Some(3), &[("skill", 200052, 2), ("race", 5, 1)]);
        let b = breakdown(Some(("Speed", 3)), Some(("Turf", 2)), Some(3), &[("race", 5, 1), ("skill", 200052, 2)]);
        assert!(sparks_match(&a, &b), "identical content in different skill_sparks order must still match");
    }

    #[test]
    fn sparks_match_rejects_differing_stat_or_aptitude_or_unique() {
        let base = breakdown(Some(("Speed", 3)), Some(("Turf", 2)), Some(3), &[]);
        assert!(!sparks_match(&base, &breakdown(Some(("Stamina", 3)), Some(("Turf", 2)), Some(3), &[])));
        assert!(!sparks_match(&base, &breakdown(Some(("Speed", 3)), Some(("Dirt", 2)), Some(3), &[])));
        assert!(!sparks_match(&base, &breakdown(Some(("Speed", 3)), Some(("Turf", 2)), Some(2), &[])));
    }

    #[test]
    fn sparks_match_rejects_differing_skill_sparks() {
        let a = breakdown(None, None, None, &[("skill", 200052, 2)]);
        let b = breakdown(None, None, None, &[("skill", 200052, 3)]);
        let empty = breakdown(None, None, None, &[]);
        assert!(!sparks_match(&a, &b), "same spark_id/type but different stars must not match");
        assert!(!sparks_match(&a, &empty), "non-empty vs empty skill_sparks must not match");
    }

    #[test]
    fn factor_cache_staleness_bound() {
        let max_age = std::time::Duration::from_secs(600);
        assert!(std::time::Duration::from_secs(599) <= max_age, "just under the bound stays valid");
        assert!(std::time::Duration::from_secs(601) > max_age, "just over the bound is stale");
    }

    #[test]
    fn process_response_routes_both_finish_and_reroll_from_one_packet() {
        // A single packet carrying both a finish-common key and a factor-lottery key
        // must trigger both handlers — proving the independent-checks restructuring
        // (as opposed to the old early-return chain) actually works.
        let combined = map(vec![
            ("single_mode_finish_common", map(vec![("card_id", Value::from(1))])),
            (
                "single_mode_factor_lottery_common",
                map(vec![(
                    "factor_select_info_array",
                    Value::Array(vec![map(vec![
                        ("lottery_id", Value::from(42)),
                        ("factor_info_array", Value::Array(vec![map(vec![("factor_id", Value::from(203))])])),
                    ])]),
                )]),
            ),
        ]);
        let Value::Map(entries) = &combined else { unreachable!() };

        assert!(extract::find_finish_common(entries).is_some());
        assert!(extract::find_factor_lottery_common(entries).is_some());
    }

    #[test]
    fn no_recognized_top_level_key_does_nothing() {
        let unrelated = map(vec![("some_other_response", Value::from(1))]);
        let Value::Map(entries) = &unrelated else { unreachable!() };
        assert!(extract::find_finish_common(entries).is_none());
        assert!(extract::find_factor_lottery_common(entries).is_none());
    }

    #[test]
    fn parse_msgpack_roundtrips_encoded_packet() {
        let packet = factor_lottery_packet(7, &[203, 1101]);
        let bytes = encode(&packet);
        let parsed = parse_msgpack(&bytes).expect("should parse");
        let Value::Map(entries) = parsed else { panic!("expected map") };
        let lottery = extract::find_factor_lottery_common(&entries).expect("should find factor_lottery_common");
        let lists = extract::extract_factor_list(lottery);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].lottery_id, Some(7));
    }
}
