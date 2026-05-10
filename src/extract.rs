use rmpv::Value;

// ── Public types ─────────────────────────────────────────────────────────────

pub struct FinishSummary {
    pub card_id:          Option<i64>,
    pub scenario_id:      Option<i64>,
    pub rarity:           Option<i64>,
    pub aptitudes:        Option<Aptitudes>,
    pub stats:            Option<Stats>,
    pub rank:             Option<String>,
    pub rating:           Option<i64>,
    pub races:            Option<i64>,
    pub wins:             Option<i64>,
    pub stat_spark:       Option<SparkEntry>,
    pub aptitude_spark:   Option<SparkEntry>,
    pub unique_spark:     Option<SparkEntry>,
    pub skill_sparks:     Vec<SkillSparkEntry>,
    pub skill_array:      Vec<SkillEntry>,
    pub support_cards:    Vec<SupportCard>,
    pub race_results:     Vec<RaceResult>,
    pub win_saddle_ids:   Vec<i64>,
    pub parents:          Vec<Parent>,
}

pub struct Stats {
    pub speed:   i64,
    pub stamina: i64,
    pub power:   i64,
    pub guts:    i64,
    pub wit:     i64,
}

pub struct Aptitudes {
    pub short:       Option<i64>,
    pub mile:        Option<i64>,
    pub middle:      Option<i64>,
    pub long:        Option<i64>,
    pub front:       Option<i64>,
    pub pace:        Option<i64>,
    pub late:        Option<i64>,
    pub end:         Option<i64>,
    pub turf:        Option<i64>,
    pub dirt:        Option<i64>,
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

pub struct SkillEntry {
    pub skill_id: i64,
    pub level:    Option<i64>,
}

pub struct SupportCard {
    pub position:          i64,
    pub support_card_id:   i64,
    pub exp:               Option<i64>,
    pub limit_break_count: Option<i64>,
}

pub struct RaceResult {
    pub turn:        Option<i64>,
    pub program_id:  i64,
    pub result_rank: Option<i64>,
}

pub struct Parent {
    pub position_id:    i64,
    pub card_id:        Option<i64>,
    pub rank:           Option<String>,
    pub stat_spark:     Option<SparkEntry>,
    pub aptitude_spark: Option<SparkEntry>,
    pub unique_spark:   Option<SparkEntry>,
    pub skill_sparks:   Vec<SkillSparkEntry>,
    pub win_saddle_ids: Vec<i64>,
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

    let card_id     = get_i64(chara_info, "card_id");
    let scenario_id = get_i64(chara_info, "scenario_id");
    let rarity      = get_i64(chara_info, "rarity");

    let aptitudes = Some(Aptitudes {
        short:  get_i64(chara_info, "proper_distance_short"),
        mile:   get_i64(chara_info, "proper_distance_mile"),
        middle: get_i64(chara_info, "proper_distance_middle"),
        long:   get_i64(chara_info, "proper_distance_long"),
        front:  get_i64(chara_info, "proper_running_style_nige"),
        pace:   get_i64(chara_info, "proper_running_style_senko"),
        late:   get_i64(chara_info, "proper_running_style_sashi"),
        end:    get_i64(chara_info, "proper_running_style_oikomi"),
        turf:   get_i64(chara_info, "proper_ground_turf"),
        dirt:   get_i64(chara_info, "proper_ground_dirt"),
    });

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

    let skill_array    = extract_skill_array(target);
    let support_cards  = extract_support_cards(target);
    let race_results   = extract_race_results(race_list);
    let win_saddle_ids = extract_i64_array(target, "win_saddle_id_array");
    let parents        = extract_parents(target);

    FinishSummary {
        card_id, scenario_id, rarity, aptitudes,
        stats, rank, rating, races, wins,
        stat_spark, aptitude_spark, unique_spark, skill_sparks,
        skill_array, support_cards, race_results, win_saddle_ids, parents,
    }
}

// ── Sub-extractors ────────────────────────────────────────────────────────────

fn extract_skill_array(target: &[(Value, Value)]) -> Vec<SkillEntry> {
    let Some(arr) = get_array(target, "skill_array") else { return Vec::new() };
    arr.iter().filter_map(|item| {
        let m = match item { Value::Map(m) => m.as_slice(), _ => return None };
        let skill_id = get_i64(m, "skill_id")?;
        let level    = get_i64(m, "level");
        Some(SkillEntry { skill_id, level })
    }).collect()
}

fn extract_support_cards(target: &[(Value, Value)]) -> Vec<SupportCard> {
    let Some(arr) = get_array(target, "support_card_list") else { return Vec::new() };
    arr.iter().filter_map(|item| {
        let m = match item { Value::Map(m) => m.as_slice(), _ => return None };
        let position        = get_i64(m, "position")?;
        let support_card_id = get_i64(m, "support_card_id")?;
        let exp               = get_i64(m, "exp");
        let limit_break_count = get_i64(m, "limit_break_count");
        Some(SupportCard { position, support_card_id, exp, limit_break_count })
    }).collect()
}

fn extract_race_results(race_list: &[Value]) -> Vec<RaceResult> {
    race_list.iter().filter_map(|item| {
        let m = match item { Value::Map(m) => m.as_slice(), _ => return None };
        let program_id  = get_i64(m, "program_id")?;
        let turn        = get_i64(m, "turn");
        let result_rank = get_i64(m, "result_rank");
        Some(RaceResult { turn, program_id, result_rank })
    }).collect()
}

fn extract_i64_array(entries: &[(Value, Value)], key: &str) -> Vec<i64> {
    get_array(entries, key)
        .map(|arr| arr.iter().filter_map(val_i64).collect())
        .unwrap_or_default()
}

fn extract_parents(target: &[(Value, Value)]) -> Vec<Parent> {
    let Some(arr) = get_array(target, "succession_chara_array") else { return Vec::new() };
    arr.iter().filter_map(|item| {
        let m = match item { Value::Map(m) => m.as_slice(), _ => return None };
        let position_id = get_i64(m, "position_id")?;
        let card_id     = get_i64(m, "card_id");
        let rank        = get_i64(m, "rank").and_then(rank_label);

        let mut stat_spark:     Option<SparkEntry>   = None;
        let mut aptitude_spark: Option<SparkEntry>   = None;
        let mut unique_spark:   Option<SparkEntry>   = None;
        let mut skill_sparks:   Vec<SkillSparkEntry> = Vec::new();

        if let Some(ids) = get_array(m, "factor_id_array") {
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

        let win_saddle_ids = extract_i64_array(m, "win_saddle_id_array");

        Some(Parent { position_id, card_id, rank, stat_spark, aptitude_spark, unique_spark, skill_sparks, win_saddle_ids })
    }).collect()
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
