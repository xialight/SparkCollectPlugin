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

#[derive(Clone)]
pub struct SparkEntry {
    pub name:  String,
    pub stars: i64,
}

#[derive(Clone, Copy)]
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

/// Decoded output of walking a list of raw `factor_id` ints through
/// `categorize_factor`. Shared by the finish-run path (`factor_id_array`,
/// bare ints) and the spark-reroll path (`factor_info_array`, `{factor_id}`
/// maps) — the two paths differ only in how the ids are pulled off the wire,
/// not in how they're categorized.
#[derive(Default)]
pub struct SparkBreakdown {
    pub stat_spark:     Option<SparkEntry>,
    pub aptitude_spark: Option<SparkEntry>,
    pub unique_spark:   Option<SparkEntry>,
    pub skill_sparks:   Vec<SkillSparkEntry>,
}

/// One candidate spark list from a `factor_select_info_array` entry within a
/// `single_mode_factor_lottery_common` packet (one reroll result).
pub struct FactorListEntry {
    pub lottery_id: Option<i64>,
    pub sparks:     SparkBreakdown,
}

// ── Entry points ─────────────────────────────────────────────────────────────

pub fn find_finish_common<'a>(entries: &'a [(Value, Value)]) -> Option<&'a [(Value, Value)]> {
    map_get(entries, "single_mode_finish_common")
        .or_else(|| map_get(entries, "data").and_then(|d| map_get(d, "single_mode_finish_common")))
}

/// The result of a single reroll action. (`single_mode_factor_select_common`,
/// the original pre-reroll list, is deliberately NOT tracked — its spark list
/// always reappears as one of this packet's entries, so it's pure duplicate
/// data.)
pub fn find_factor_lottery_common<'a>(entries: &'a [(Value, Value)]) -> Option<&'a [(Value, Value)]> {
    map_get(entries, "single_mode_factor_lottery_common")
        .or_else(|| map_get(entries, "data").and_then(|d| map_get(d, "single_mode_factor_lottery_common")))
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

    let breakdown = categorize_factor_ids(extract_factor_ids(target, "factor_info_array").into_iter());

    let skill_array    = extract_skill_array(target);
    let support_cards  = extract_support_cards(target);
    let race_results   = extract_race_results(race_list);
    let win_saddle_ids = extract_i64_array(target, "win_saddle_id_array");
    let parents        = extract_parents(target);

    FinishSummary {
        card_id, scenario_id, rarity,
        stats, aptitudes, rank, rating, races, wins,
        stat_spark: breakdown.stat_spark,
        aptitude_spark: breakdown.aptitude_spark,
        unique_spark: breakdown.unique_spark,
        skill_sparks: breakdown.skill_sparks,
        skill_array, support_cards, race_results, win_saddle_ids, parents,
    }
}

/// Reads every candidate spark list out of a `single_mode_factor_select_common` /
/// `single_mode_factor_lottery_common` packet. Returns one entry per element of
/// `factor_select_info_array`, even entries whose sparks end up empty — deciding
/// whether an empty entry is worth submitting is the caller's policy, not this
/// function's.
pub fn extract_factor_list(packet: &[(Value, Value)]) -> Vec<FactorListEntry> {
    let Some(arr) = get_array(packet, "factor_select_info_array") else { return Vec::new() };
    arr.iter().filter_map(|item| {
        let m = match item { Value::Map(m) => m.as_slice(), _ => return None };
        let lottery_id = get_i64(m, "lottery_id");
        let ids = extract_factor_ids(m, "factor_info_array");
        Some(FactorListEntry { lottery_id, sparks: categorize_factor_ids(ids.into_iter()) })
    }).collect()
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

        let breakdown = categorize_factor_ids(extract_factor_ids(m, "factor_info_array").into_iter());

        let win_saddle_ids = extract_i64_array(m, "win_saddle_id_array");

        Some(Parent {
            position_id, card_id, rank,
            stat_spark: breakdown.stat_spark,
            aptitude_spark: breakdown.aptitude_spark,
            unique_spark: breakdown.unique_spark,
            skill_sparks: breakdown.skill_sparks,
            win_saddle_ids,
        })
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

/// Walks a list of raw `factor_id` ints, keeping the first stat/aptitude/unique
/// spark seen (the game only ever grants one of each per character) and
/// collecting every skill spark (white sparks are multi-valued).
fn categorize_factor_ids(ids: impl Iterator<Item = i64>) -> SparkBreakdown {
    let mut breakdown = SparkBreakdown::default();
    for id in ids {
        match categorize_factor(id) {
            Factor::Stat { name, stars } if breakdown.stat_spark.is_none() => {
                breakdown.stat_spark = Some(SparkEntry { name: name.into(), stars });
            }
            Factor::Aptitude { name, stars } if breakdown.aptitude_spark.is_none() => {
                breakdown.aptitude_spark = Some(SparkEntry { name: name.into(), stars });
            }
            Factor::Unique { stars } if breakdown.unique_spark.is_none() => {
                breakdown.unique_spark = Some(SparkEntry { name: "Character Factor".into(), stars });
            }
            Factor::Skill(entry) => breakdown.skill_sparks.push(entry),
            _ => {}
        }
    }
    breakdown
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

/// Extracts every `factor_id` out of a `factor_info_array`-shaped list — an
/// array of `{factor_id, level}` objects (used by a trained/succession
/// chara's own committed sparks, and by each reroll candidate's spark list).
/// Not a flat array of ints, despite the sibling `single_mode_factor_lottery_common`
/// top-level key being named `factor_id_array` in some contexts — the
/// per-chara committed spark list is always object-shaped.
fn extract_factor_ids(entries: &[(Value, Value)], key: &str) -> Vec<i64> {
    get_array(entries, key).unwrap_or(&[]).iter().filter_map(|v| match v {
        Value::Map(fm) => get_i64(fm, "factor_id"),
        _ => None,
    }).collect()
}

fn val_i64(v: &Value) -> Option<i64> { v.as_i64() }

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (Value::from(k), v)).collect())
    }

    fn factor_info(ids: &[i64]) -> Value {
        Value::Array(ids.iter().map(|&id| map(vec![("factor_id", Value::from(id))])).collect())
    }

    fn select_entry(lottery_id: Option<i64>, ids: &[i64]) -> Value {
        let mut pairs = vec![("factor_info_array", factor_info(ids))];
        if let Some(lid) = lottery_id {
            pairs.push(("lottery_id", Value::from(lid)));
        }
        map(pairs)
    }

    #[test]
    fn extract_factor_list_covers_every_categorize_bucket() {
        // stat=203 (Stamina/3), aptitude=1101 (Turf/1), race=1_000_203 (spark_id 2002, 3 stars),
        // skill=2_000_412 (skill), scenario=3_000_203, unique=10_000_103 (3 stars)
        let ids = [203, 1101, 1_000_203, 2_000_412, 3_000_203, 10_000_103];
        let packet_top = map(vec![(
            "factor_select_info_array",
            Value::Array(vec![select_entry(Some(1), &ids), select_entry(Some(2), &[999_999])]),
        )]);
        let Value::Map(entries) = packet_top else { unreachable!() };

        let lists = extract_factor_list(&entries);
        assert_eq!(lists.len(), 2);

        let first = &lists[0];
        assert_eq!(first.lottery_id, Some(1));
        assert_eq!(first.sparks.stat_spark.as_ref().map(|s| (s.name.as_str(), s.stars)), Some(("Stamina", 3)));
        assert_eq!(first.sparks.aptitude_spark.as_ref().map(|s| (s.name.as_str(), s.stars)), Some(("Turf", 1)));
        assert_eq!(first.sparks.unique_spark.as_ref().map(|s| s.stars), Some(3));
        assert_eq!(first.sparks.skill_sparks.len(), 3);
        assert!(first.sparks.skill_sparks.iter().any(|s| s.spark_type == "race"));
        assert!(first.sparks.skill_sparks.iter().any(|s| s.spark_type == "skill"));
        assert!(first.sparks.skill_sparks.iter().any(|s| s.spark_type == "scenario"));

        let second = &lists[1];
        assert_eq!(second.lottery_id, Some(2));
        assert!(second.sparks.stat_spark.is_none());
        assert!(second.sparks.aptitude_spark.is_none());
        assert!(second.sparks.unique_spark.is_none());
        assert!(second.sparks.skill_sparks.is_empty());
    }

    #[test]
    fn find_factor_lottery_common_top_level_and_nested() {
        let top = map(vec![("single_mode_factor_lottery_common", map(vec![("a", Value::from(1))]))]);
        let Value::Map(entries) = top else { unreachable!() };
        assert!(find_factor_lottery_common(&entries).is_some());

        let nested = map(vec![("data", map(vec![("single_mode_factor_lottery_common", map(vec![("a", Value::from(1))]))]))]);
        let Value::Map(entries) = nested else { unreachable!() };
        assert!(find_factor_lottery_common(&entries).is_some());
    }

    #[test]
    fn find_factor_lottery_common_returns_none_when_absent() {
        let top = map(vec![("unrelated_key", Value::from(1))]);
        let Value::Map(entries) = top else { unreachable!() };
        assert!(find_factor_lottery_common(&entries).is_none());
    }

    #[test]
    fn extract_finish_summary_reads_object_shaped_factor_info_array() {
        // The committed spark list is `factor_info_array` — an array of
        // `{factor_id, level}` objects, same shape as a reroll candidate's
        // own list — not a flat `factor_id_array` of ints.
        let target = map(vec![
            ("card_id", Value::from(12345)),
            ("factor_info_array", factor_info(&[203, 1101, 2_000_412])),
        ]);
        let Value::Map(entries) = target else { unreachable!() };
        let summary = extract_finish_summary(&entries);
        assert_eq!(summary.card_id, Some(12345));
        assert_eq!(summary.stat_spark.as_ref().map(|s| s.name.as_str()), Some("Stamina"));
        assert_eq!(summary.aptitude_spark.as_ref().map(|s| s.name.as_str()), Some("Turf"));
        assert_eq!(summary.skill_sparks.len(), 1);
    }

    #[test]
    fn extract_parents_reads_object_shaped_factor_info_array() {
        let target = map(vec![(
            "succession_chara_array",
            Value::Array(vec![map(vec![
                ("position_id", Value::from(10)),
                ("card_id", Value::from(101401)),
                ("factor_info_array", factor_info(&[203, 1101])),
            ])]),
        )]);
        let Value::Map(entries) = target else { unreachable!() };
        let parents = extract_parents(&entries);
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].stat_spark.as_ref().map(|s| s.name.as_str()), Some("Stamina"));
        assert_eq!(parents[0].aptitude_spark.as_ref().map(|s| s.name.as_str()), Some("Turf"));
    }
}
