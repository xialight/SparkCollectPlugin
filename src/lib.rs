mod extract;
mod worker;

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;
use std::sync::Mutex;

use hachimi_plugin_sdk::{
    api::{Hachimi, HachimiApi},
    hachimi_plugin,
    il2cpp::{helpers::Array, types::{Il2CppArray, Il2CppClass, Il2CppImage, Il2CppObject}},
    sys::InitResult,
};
use lazy_static::lazy_static;
use log::{error, info};

const VERSION: &str = "0.2.0";
const CONFIG_DIR:  &str = "SparkCollectPlugin";
const CONFIG_FILE: &str = "config.properties";
const DEFAULT_COOLDOWN_SECS: u64 = 30;

// Set SPARK_WORKER_URL before building:
//   SPARK_WORKER_URL=https://your-api.workers.dev cargo build --release
const WORKER_URL: &str = env!("SPARK_WORKER_URL");

static mut API: Option<HachimiApi> = None;
static mut DECOMPRESS_ORIG: usize  = 0;

lazy_static! {
    static ref CONFIG: HashMap<String, String> = load_config();
    static ref LAST_NOTIFIED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
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

    if let Some(trampoline) = interceptor.hook(addr, hook_decompress_response as _) {
        unsafe { DECOMPRESS_ORIG = trampoline; }
    }

    info!("SparkCollectPlugin {VERSION} loaded — watching for career completions");
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
    let Some(finish_common) = extract::find_finish_common(entries) else {
        return;
    };

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

    let summary = extract::extract_finish_summary(finish_common);

    let has_sparks = summary.stat_spark.is_some()
        || summary.aptitude_spark.is_some()
        || !summary.skill_sparks.is_empty();

    if !has_sparks {
        info!("Career finish detected but no sparks — skipping (run not complete)");
        return;
    }

    if cfg("debug_raw").as_deref() == Some("true") {
        debug_save_json(&value);
    }

    let token = match cfg("token") {
        Some(t) => t,
        None => {
            error!("No token configured in {CONFIG_DIR}/{CONFIG_FILE} — cannot submit run");
            return;
        }
    };

    std::thread::spawn(move || {
        worker::submit_run(WORKER_URL, &token, &summary);
    });
}

fn debug_save_json(value: &rmpv::Value) {
    const LOG_DIR: &str = "SparkCollectPlugin/logs";
    if let Err(e) = std::fs::create_dir_all(LOG_DIR) {
        error!("debug_raw: failed to create logs dir: {e}");
        return;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = format!("{LOG_DIR}/{ts}.json");
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
          debug_raw=false\n",
    );
}
