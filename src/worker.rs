use crate::extract::FinishSummary;
use log::{error, info};

/// POST the career-finish summary directly to the SparkTracker worker.
pub fn submit_run(worker_url: &str, token: &str, summary: &FinishSummary) {
    let stat     = match &summary.stat_spark     { Some(s) => s, None => { error!("submit_run: missing stat spark — skipping"); return; } };
    let aptitude = match &summary.aptitude_spark { Some(s) => s, None => { error!("submit_run: missing aptitude spark — skipping"); return; } };
    let unique_stars = summary.unique_spark.as_ref().map(|s| s.stars).unwrap_or(1);
    let card_id      = summary.card_id.unwrap_or(0);
    let rank         = escape_json(summary.rank.as_deref().unwrap_or("G"));
    let rating       = opt_i64_json(summary.rating);
    let races        = opt_i64_json(summary.races);
    let wins         = opt_i64_json(summary.wins);

    let stats_json = match &summary.stats {
        Some(st) => format!(
            r#"{{"spd":{},"sta":{},"pow":{},"gut":{},"wit":{}}}"#,
            st.speed, st.stamina, st.power, st.guts, st.wit
        ),
        None => r#"{"spd":0,"sta":0,"pow":0,"gut":0,"wit":0}"#.into(),
    };

    let skill_sparks_json: Vec<String> = summary.skill_sparks.iter().map(|s| {
        format!(r#"{{"spark_type":"{}","spark_id":{},"stars":{}}}"#, s.spark_type, s.spark_id, s.stars)
    }).collect();
    let skill_sparks_str = skill_sparks_json.join(",");

    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let message_id = format!("plugin_{ts_ms}");

    let stat_name = escape_json(&stat.name);
    let apt_name  = escape_json(&aptitude.name);

    let payload = format!(
        r#"{{"token":"{token}","message_id":"{message_id}","character_id":{card_id},"rank":"{rank}","rating":{rating},"races":{races},"wins":{wins},"obtained_at_unix_ms":{ts_ms},"stats":{stats_json},"stat_spark":{{"name":"{stat_name}","stars":{}}},"aptitude_spark":{{"name":"{apt_name}","stars":{}}},"unique_spark_stars":{unique_stars},"skill_sparks":[{skill_sparks_str}]}}"#,
        stat.stars, aptitude.stars
    );

    let url = format!("{worker_url}/api/plugin/run");
    match ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("User-Agent", "SparkCollectPlugin/0.2")
        .send_string(&payload)
    {
        Ok(resp) => info!("worker: run submitted ({})", resp.status()),
        Err(e)   => error!("worker: submit failed: {e}"),
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
