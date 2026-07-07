use crate::extract::{FinishSummary, Parent, SparkBreakdown, SparkEntry};
use log::{error, info};

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// POST the career-finish summary directly to the SparkTracker worker.
/// Common case: no spark reroll happened this career, so the committed
/// breakdown (already on `summary`) is the run's only submission.
pub fn submit_run(worker_url: &str, token: &str, summary: &FinishSummary) {
    let sparks = SparkBreakdown {
        stat_spark:     summary.stat_spark.clone(),
        aptitude_spark: summary.aptitude_spark.clone(),
        unique_spark:   summary.unique_spark.clone(),
        skill_sparks:   summary.skill_sparks.clone(),
    };
    build_and_post(worker_url, token, summary, &sparks, None, true, None, now_ms());
}

/// POST one variant of a career finish that had at least one spark-reroll
/// candidate cached: either one cached candidate (`lottery_id = Some(..)`),
/// or — if none of the cached candidates matched the game's own committed
/// `factor_id_array` — the committed breakdown itself (`lottery_id = None`,
/// `is_selected = true`, `variant_tag = Some("committed")`).
///
/// `obtained_at_ms` is computed ONCE by the caller (`lib.rs`) and shared
/// across every variant of the same finish event — both so all N rows record
/// the exact same real-world moment, and so the worker's rate limiter can
/// tell this burst apart from a genuinely different finish.
pub fn submit_run_variant(
    worker_url: &str,
    token: &str,
    summary: &FinishSummary,
    sparks: &SparkBreakdown,
    lottery_id: Option<i64>,
    is_selected: bool,
    variant_tag: Option<String>,
    obtained_at_ms: u128,
) {
    build_and_post(worker_url, token, summary, sparks, lottery_id, is_selected, variant_tag, obtained_at_ms);
}

fn build_and_post(
    worker_url: &str,
    token: &str,
    summary: &FinishSummary,
    sparks: &SparkBreakdown,
    lottery_id: Option<i64>,
    is_selected: bool,
    variant_tag: Option<String>,
    obtained_at_ms: u128,
) {
    let stat     = match &sparks.stat_spark     { Some(s) => s, None => { error!("submit_run: missing stat spark (lottery_id={lottery_id:?}) — skipping"); return; } };
    let aptitude = match &sparks.aptitude_spark { Some(s) => s, None => { error!("submit_run: missing aptitude spark (lottery_id={lottery_id:?}) — skipping"); return; } };

    let card_id  = summary.card_id.unwrap_or(0);
    let rank     = escape_json(summary.rank.as_deref().unwrap_or("G"));
    let rating   = opt_i64_json(summary.rating);
    let races    = opt_i64_json(summary.races);
    let wins     = opt_i64_json(summary.wins);

    // Unchanged shape for the common case (variant_tag=None): `plugin_{ts}`.
    let message_id = match &variant_tag {
        Some(tag) => format!("plugin_{obtained_at_ms}_{tag}"),
        None      => format!("plugin_{obtained_at_ms}"),
    };

    // ── Core spark fields ─────────────────────────────────────────────────────
    let stat_name = escape_json(&stat.name);
    let apt_name  = escape_json(&aptitude.name);
    let unique_stars_json = sparks.unique_spark.as_ref()
        .map(|s| s.stars.to_string())
        .unwrap_or_else(|| "null".into());

    // ── Stats ─────────────────────────────────────────────────────────────────
    let stats_json = match &summary.stats {
        Some(st) => format!(
            r#"{{"spd":{},"sta":{},"pow":{},"gut":{},"wit":{}}}"#,
            st.speed, st.stamina, st.power, st.guts, st.wit
        ),
        None => r#"{"spd":0,"sta":0,"pow":0,"gut":0,"wit":0}"#.into(),
    };

    // ── Aptitudes ─────────────────────────────────────────────────────────────
    let aptitudes_json = match &summary.aptitudes {
        Some(a) => format!(
            r#"{{"short":{},"mile":{},"middle":{},"long":{},"front":{},"pace":{},"late":{},"end":{},"turf":{},"dirt":{}}}"#,
            opt_i64_json(a.short), opt_i64_json(a.mile),   opt_i64_json(a.middle), opt_i64_json(a.long),
            opt_i64_json(a.front), opt_i64_json(a.pace),   opt_i64_json(a.late),   opt_i64_json(a.end),
            opt_i64_json(a.turf),  opt_i64_json(a.dirt)
        ),
        None => "null".into(),
    };

    // ── White sparks (this variant's own, not necessarily summary's) ─────────
    let skill_sparks_str = skill_sparks_json(&sparks.skill_sparks);

    // ── skill_array ───────────────────────────────────────────────────────────
    let skill_array_json: Vec<String> = summary.skill_array.iter().map(|s| {
        format!(r#"{{"skill_id":{},"level":{}}}"#, s.skill_id, opt_i64_json(s.level))
    }).collect();
    let skill_array_str = skill_array_json.join(",");

    // ── Support cards ─────────────────────────────────────────────────────────
    let support_cards_json: Vec<String> = summary.support_cards.iter().map(|c| {
        format!(
            r#"{{"position":{},"support_card_id":{},"exp":{},"limit_break_count":{}}}"#,
            c.position, c.support_card_id, opt_i64_json(c.exp), opt_i64_json(c.limit_break_count)
        )
    }).collect();
    let support_cards_str = support_cards_json.join(",");

    // ── Race results ──────────────────────────────────────────────────────────
    let race_results_json: Vec<String> = summary.race_results.iter().map(|r| {
        format!(
            r#"{{"turn":{},"program_id":{},"result_rank":{}}}"#,
            opt_i64_json(r.turn), r.program_id, opt_i64_json(r.result_rank)
        )
    }).collect();
    let race_results_str = race_results_json.join(",");

    // ── Win saddle IDs ────────────────────────────────────────────────────────
    let win_saddle_str = summary.win_saddle_ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    // ── Parents ───────────────────────────────────────────────────────────────
    let parents_json: Vec<String> = summary.parents.iter().map(|p| parent_json(p)).collect();
    let parents_str = parents_json.join(",");

    let scenario_id_json = opt_i64_json(summary.scenario_id);
    let rarity_json      = opt_i64_json(summary.rarity);
    let lottery_id_json  = opt_i64_json(lottery_id);
    let is_selected_json = if is_selected { "true" } else { "false" };

    let payload = format!(
        r#"{{"token":"{token}","message_id":"{message_id}","kind":"finish","character_id":{card_id},"lottery_id":{lottery_id_json},"is_selected":{is_selected_json},"rank":"{rank}","rating":{rating},"races":{races},"wins":{wins},"obtained_at_unix_ms":{obtained_at_ms},"scenario_id":{scenario_id_json},"rarity":{rarity_json},"stats":{stats_json},"aptitudes":{aptitudes_json},"stat_spark":{{"name":"{stat_name}","stars":{}}},"aptitude_spark":{{"name":"{apt_name}","stars":{}}},"unique_spark_stars":{unique_stars_json},"skill_sparks":[{skill_sparks_str}],"skill_array":[{skill_array_str}],"support_cards":[{support_cards_str}],"race_results":[{race_results_str}],"win_saddle_ids":[{win_saddle_str}],"parents":[{parents_str}]}}"#,
        stat.stars, aptitude.stars
    );

    post(worker_url, &payload, "finish", lottery_id);
}

fn post(worker_url: &str, payload: &str, kind: &str, lottery_id: Option<i64>) {
    let url = format!("{worker_url}/api/plugin/run");
    match ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("User-Agent", "SparkCollectPlugin/0.3")
        .send_string(payload)
    {
        Ok(resp) => info!("worker: {kind} run submitted (lottery_id={lottery_id:?}, status={})", resp.status()),
        Err(e)   => error!("worker: {kind} submit failed (lottery_id={lottery_id:?}): {e}"),
    }
}

fn skill_sparks_json(sparks: &[crate::extract::SkillSparkEntry]) -> String {
    sparks.iter().map(|s| {
        format!(r#"{{"spark_type":"{}","spark_id":{},"stars":{}}}"#, s.spark_type, s.spark_id, s.stars)
    }).collect::<Vec<_>>().join(",")
}

fn parent_json(p: &Parent) -> String {
    let card_id_json = opt_i64_json(p.card_id);
    let rank_json = match &p.rank {
        Some(r) => format!("\"{}\"", escape_json(r)),
        None    => "null".into(),
    };
    let stat_spark_json = spark_entry_json(p.stat_spark.as_ref());
    let apt_spark_json  = spark_entry_json(p.aptitude_spark.as_ref());
    let uniq_spark_json = match &p.unique_spark {
        Some(s) => s.stars.to_string(),
        None    => "null".into(),
    };
    let skill_sparks_str = skill_sparks_json(&p.skill_sparks);
    let saddle_str = p.win_saddle_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
    format!(
        r#"{{"position_id":{},"card_id":{},"rank":{},"stat_spark":{},"aptitude_spark":{},"unique_spark_stars":{},"skill_sparks":[{}],"win_saddle_ids":[{}]}}"#,
        p.position_id, card_id_json, rank_json,
        stat_spark_json, apt_spark_json, uniq_spark_json,
        skill_sparks_str, saddle_str
    )
}

fn spark_entry_json(entry: Option<&SparkEntry>) -> String {
    match entry {
        Some(s) => format!(r#"{{"name":"{}","stars":{}}}"#, escape_json(&s.name), s.stars),
        None    => "null".into(),
    }
}

fn opt_i64_json(v: Option<i64>) -> String {
    match v { Some(n) => n.to_string(), None => "null".into() }
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c    => out.push(c),
        }
    }
    out
}
