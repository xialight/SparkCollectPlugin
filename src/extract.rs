use rmpv::Value;

// ── Public types ─────────────────────────────────────────────────────────────

pub struct FinishSummary {
    pub card_id:          Option<i64>,
    pub stats:            Option<Stats>,
    pub rank:             Option<String>,
    pub rating:           Option<i64>,
    pub races:            Option<i64>,
    pub wins:             Option<i64>,
    pub stat_spark:       Option<SparkEntry>,     // first blue (stat) spark
    pub aptitude_spark:   Option<SparkEntry>,     // first pink (aptitude) spark
    pub unique_spark:     Option<SparkEntry>,     // first green (character factor) spark
    pub skill_sparks:     Vec<SkillSparkEntry>,   // all white sparks (race / skill / scenario)
}

pub struct Stats {
    pub speed:   i64,
    pub stamina: i64,
    pub power:   i64,
    pub guts:    i64,
    pub wit:     i64,
}

pub struct SparkEntry {
    pub name:  String,
    pub stars: i64,
}

pub struct SkillSparkEntry {
    pub spark_type: &'static str,  // "race" | "skill" | "scenario"
    pub spark_id:   i64,
    pub stars:      i64,
}

// ── Entry points ─────────────────────────────────────────────────────────────

pub fn find_finish_common<'a>(entries: &'a [(Value, Value)]) -> Option<&'a [(Value, Value)]> {
    map_get(entries, "single_mode_finish_common")
        .or_else(|| map_get(entries, "data").and_then(|d| map_get(d, "single_mode_finish_common")))
}

pub fn extract_finish_summary(finish_common: &[(Value, Value)]) -> FinishSummary {
    let target_id = get_i64(finish_common, "trained_chara_id");
    let target = find_target_chara(finish_common, target_id)
        .or_else(|| map_get(finish_common, "chara_info"))
        .unwrap_or(finish_common);

    let chara_info = map_get(finish_common, "chara_info").unwrap_or(target);

    let card_id = get_i64(chara_info, "card_id");

    let stats = {
        let speed   = get_i64(chara_info, "speed");
        let stamina = get_i64(chara_info, "stamina");
        let power   = get_i64(chara_info, "power");
        let guts    = get_i64(chara_info, "guts");
        let wit     = get_i64(chara_info, "wiz");
        match (speed, stamina, power, guts, wit) {
            (Some(sp), Some(st), Some(pw), Some(gu), Some(wi)) =>
                Some(Stats { speed: sp, stamina: st, power: pw, guts: gu, wit: wi }),
            _ => None,
        }
    };

    let rank   = get_i64(target, "rank").and_then(rank_label);
    let rating = get_i64(target, "rank_score");

    let race_list = get_array(target, "race_result_list").unwrap_or(&[]);
    let races     = Some(race_list.len() as i64);
    let wins      = get_i64(target, "wins").or_else(|| {
        Some(
            race_list
                .iter()
                .filter(|r| matches!(r, Value::Map(m) if get_i64(m, "result_rank") == Some(1)))
                .count() as i64,
        )
    });

    let mut stat_spark:     Option<SparkEntry>    = None;
    let mut aptitude_spark: Option<SparkEntry>    = None;
    let mut unique_spark:   Option<SparkEntry>    = None;
    let mut skill_sparks:   Vec<SkillSparkEntry>  = Vec::new();

    if let Some(ids) = get_array(target, "factor_id_array") {
        for id_val in ids {
            if let Some(id) = val_i64(id_val) {
                match categorize_factor(id) {
                    Factor::Stat { name, stars } if stat_spark.is_none() => {
                        stat_spark = Some(SparkEntry { name: name.into(), stars });
                    }
                    Factor::Aptitude { name, stars } if aptitude_spark.is_none() => {
                        aptitude_spark = Some(SparkEntry { name: name.into(), stars });
                    }
                    Factor::Unique { stars } if unique_spark.is_none() => {
                        unique_spark = Some(SparkEntry { name: "Character Factor".into(), stars });
                    }
                    Factor::Skill(entry) => skill_sparks.push(entry),
                    _ => {}
                }
            }
        }
    }

    FinishSummary { card_id, stats, rank, rating, races, wins, stat_spark, aptitude_spark, unique_spark, skill_sparks }
}

// ── Factor decoding ───────────────────────────────────────────────────────────

enum Factor {
    Stat     { name: &'static str, stars: i64 },
    Aptitude { name: &'static str, stars: i64 },
    Unique   { stars: i64 },
    Skill(SkillSparkEntry),
    Unknown,
}

fn categorize_factor(v: i64) -> Factor {
    let stars = if v >= 10_000_000 { v % 100 } else { v % 10 };

    if (100..600).contains(&v) {
        let name = match v / 100 {
            1 => "Speed", 2 => "Stamina", 3 => "Power", 4 => "Guts", 5 => "Wit",
            _ => return Factor::Unknown,
        };
        return Factor::Stat { name, stars };
    }

    if (1_000..4_000).contains(&v) {
        let name = match v / 100 {
            11 => "Turf",         12 => "Dirt",
            21 => "Front Runner", 22 => "Pace Chaser", 23 => "Late Surger", 24 => "End Closer",
            31 => "Sprint",       32 => "Mile",         33 => "Medium",      34 => "Long",
            _ => return Factor::Unknown,
        };
        return Factor::Aptitude { name, stars };
    }

    if (1_000_000..2_000_000).contains(&v) {
        let spark_id = (v - 1_000_000) / 100;
        return Factor::Skill(SkillSparkEntry { spark_type: "race", spark_id, stars });
    }

    if (2_000_000..3_000_000).contains(&v) {
        let spark_id = (v / 100) * 10 + v % 100;
        return Factor::Skill(SkillSparkEntry { spark_type: "skill", spark_id, stars });
    }

    if (3_000_000..4_000_000).contains(&v) {
        let spark_id = (v - 3_000_000) / 100;
        return Factor::Skill(SkillSparkEntry { spark_type: "scenario", spark_id, stars });
    }

    if v >= 10_000_000 {
        return Factor::Unique { stars };
    }

    Factor::Unknown
}

fn find_target_chara<'a>(
    finish_common: &'a [(Value, Value)],
    target_id: Option<i64>,
) -> Option<&'a [(Value, Value)]> {
    let id   = target_id?;
    let list = get_array(finish_common, "trained_chara")?;
    list.iter().find_map(|item| match item {
        Value::Map(m) if get_i64(m, "trained_chara_id") == Some(id) => Some(m.as_slice()),
        _ => None,
    })
}

fn rank_label(rank: i64) -> Option<String> {
    if rank < 1 { return None; }
    const BASE: &[&str] = &[
        "G", "G+", "F", "F+", "E", "E+", "D", "D+",
        "C", "C+", "B", "B+", "A", "A+", "S", "S+", "SS", "SS+",
    ];
    if rank <= BASE.len() as i64 {
        return Some(BASE[(rank - 1) as usize].into());
    }
    const U: &[&str] = &["G", "F", "E", "D", "C", "B", "A", "S", "SS"];
    let offset = rank - 19;
    let tier   = (offset / 10) as usize;
    let sub    = offset % 10;
    if tier >= U.len() { return None; }
    Some(if sub == 0 { format!("U{}", U[tier]) } else { format!("U{}{sub}", U[tier]) })
}

// ── msgpack helpers ───────────────────────────────────────────────────────────

pub fn map_get<'a>(entries: &'a [(Value, Value)], key: &str) -> Option<&'a [(Value, Value)]> {
    entries.iter().find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| match v { Value::Map(m) => Some(m.as_slice()), _ => None })
}

fn get_i64(entries: &[(Value, Value)], key: &str) -> Option<i64> {
    entries.iter().find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| val_i64(v))
}

fn get_array<'a>(entries: &'a [(Value, Value)], key: &str) -> Option<&'a [Value]> {
    entries.iter().find(|(k, _)| k.as_str() == Some(key))
        .and_then(|(_, v)| match v { Value::Array(a) => Some(a.as_slice()), _ => None })
}

fn val_i64(v: &Value) -> Option<i64> { v.as_i64() }
