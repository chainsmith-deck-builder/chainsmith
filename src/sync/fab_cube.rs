//! Pull and parse data from the upstream
//! `the-fab-cube/flesh-and-blood-cards` GitHub repo into the engine's domain
//! types.
//!
//! Wire types ([`CardJson`], [`PrintingJson`], etc.) mirror the upstream JSON
//! shape verbatim — strings for numeric fields, flat type arrays. Conversion
//! functions split them into the engine's structured `Card` / `Printing`
//! shapes. The IO entry points ([`fetch_from_upstream`]) are async and sit at
//! the top of the file; the rest is pure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;

use crate::domain::card::{
    Card, CardType, Class, Edition, EssenceGrant, Foiling, FormatStatus, HeroFacts, HeroKind,
    Keyword, LegalitySummary, Printing, Rarity, Talent, Treatment, WeaponGrip,
};
use crate::domain::catalog::Catalog;
use crate::domain::format::classic_constructed::{BannedEntry, LivingLegendEntry};
use crate::domain::ids::{CardId, HeroMoniker, PrintingId, SetCode};

const UPSTREAM_REPO: &str = "https://raw.githubusercontent.com/the-fab-cube/flesh-and-blood-cards";

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cache IO error: {0}")]
    Cache(#[from] std::io::Error),
}

// ---- wire types ----

#[derive(Debug, Clone, Deserialize)]
pub struct CardJson {
    pub unique_id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub pitch: String,
    #[serde(default)]
    pub cost: String,
    #[serde(default)]
    pub power: String,
    #[serde(default)]
    pub defense: String,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub intelligence: String,
    #[serde(default)]
    pub arcane: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub card_keywords: Vec<String>,
    pub functional_text: Option<String>,
    pub type_text: Option<String>,

    // Format flags. Default to false on missing fields so partial upstream
    // data does not fail to parse.
    #[serde(default)]
    pub blitz_legal: bool,
    #[serde(default)]
    pub cc_legal: bool,
    #[serde(default)]
    pub commoner_legal: bool,
    #[serde(default)]
    pub ll_legal: bool,
    #[serde(default)]
    pub silver_age_legal: bool,
    #[serde(default)]
    pub blitz_living_legend: bool,
    #[serde(default)]
    pub cc_living_legend: bool,
    #[serde(default)]
    pub blitz_banned: bool,
    #[serde(default)]
    pub cc_banned: bool,
    #[serde(default)]
    pub commoner_banned: bool,
    #[serde(default)]
    pub ll_banned: bool,
    #[serde(default)]
    pub silver_age_banned: bool,
    #[serde(default)]
    pub upf_banned: bool,
    #[serde(default)]
    pub blitz_suspended: bool,
    #[serde(default)]
    pub cc_suspended: bool,
    #[serde(default)]
    pub commoner_suspended: bool,
    #[serde(default)]
    pub ll_restricted: bool,

    #[serde(default)]
    pub printings: Vec<PrintingJson>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrintingJson {
    pub unique_id: String,
    #[serde(default)]
    pub set_id: String,
    /// Collector number e.g. "MST131".
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub edition: String,
    #[serde(default)]
    pub foiling: String,
    #[serde(default)]
    pub rarity: String,
    #[serde(default)]
    pub artists: Vec<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LegalityEntryJson {
    pub card_unique_id: String,
    #[serde(default)]
    pub status_active: bool,
    pub date_announced: Option<String>,
    pub date_in_effect: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetJson {
    pub id: String,
    #[serde(default)]
    pub printings: Vec<SetPrintingJson>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetPrintingJson {
    pub initial_release_date: Option<String>,
}

// ---- helpers ----

fn placeholder_release_date() -> NaiveDate {
    // FaB Alpha (Welcome to Rathe) release. Used when set release data is
    // missing — non-CC formats don't depend on this.
    NaiveDate::from_ymd_opt(2019, 10, 11).expect("static date is valid")
}

fn parse_iso_date(s: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).date_naive())
}

fn parse_u8(s: &str) -> Option<u8> {
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

fn parse_u16(s: &str) -> Option<u16> {
    if s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

#[derive(Debug, Default)]
struct ClassifiedTypes {
    card_types: Vec<CardType>,
    classes: Vec<Class>,
    talents: Vec<Talent>,
    subtypes: Vec<String>,
    /// Set if "1H" or "2H" appears in types (weapon grip).
    weapon_grip: Option<WeaponGrip>,
    /// Set if "Young" or "Pit-Fighter" appears in types (hero kind override).
    hero_kind: Option<HeroKind>,
    has_action: bool,
    has_attack: bool,
}

/// Split the upstream's flat `types` array into our structured categories.
///
/// FaB upstream stores everything (card type, class, talent, subtype, weapon
/// grip, hero variant marker) as plain strings in one array. We classify each
/// against a known list; anything unknown is preserved as a subtype so the
/// engine doesn't lose data on upstream evolution.
fn classify_types(types: &[String]) -> ClassifiedTypes {
    let mut out = ClassifiedTypes::default();

    for t in types {
        match t.as_str() {
            "Hero" => out.card_types.push(CardType::Hero),
            "Action" => out.has_action = true,
            "Attack" => out.has_attack = true,
            "Attack Reaction" => out.card_types.push(CardType::AttackReaction),
            "Defense Reaction" => out.card_types.push(CardType::DefenseReaction),
            "Instant" => out.card_types.push(CardType::Instant),
            "Equipment" => out.card_types.push(CardType::Equipment),
            "Weapon" => out.card_types.push(CardType::Weapon),
            "Mentor" => out.card_types.push(CardType::Mentor),
            "Resource" => out.card_types.push(CardType::Resource),
            "Token" => out.card_types.push(CardType::Token),

            "Brute" => out.classes.push(Class::Brute),
            "Guardian" => out.classes.push(Class::Guardian),
            "Illusionist" => out.classes.push(Class::Illusionist),
            "Mechanologist" => out.classes.push(Class::Mechanologist),
            "Merchant" => out.classes.push(Class::Merchant),
            "Ninja" => out.classes.push(Class::Ninja),
            "Ranger" => out.classes.push(Class::Ranger),
            "Runeblade" => out.classes.push(Class::Runeblade),
            "Shapeshifter" => out.classes.push(Class::Shapeshifter),
            "Warrior" => out.classes.push(Class::Warrior),
            "Wizard" => out.classes.push(Class::Wizard),
            "Bard" => out.classes.push(Class::Bard),
            "Assassin" => out.classes.push(Class::Assassin),

            "Draconic" => out.talents.push(Talent::Draconic),
            "Earth" => out.talents.push(Talent::Earth),
            "Elemental" => out.talents.push(Talent::Elemental),
            "Ice" => out.talents.push(Talent::Ice),
            "Light" => out.talents.push(Talent::Light),
            "Lightning" => out.talents.push(Talent::Lightning),
            "Shadow" => out.talents.push(Talent::Shadow),
            "Wind" => out.talents.push(Talent::Wind),

            "Young" => out.hero_kind = Some(HeroKind::Young),
            "Pit-Fighter" => out.hero_kind = Some(HeroKind::PitFighter),

            "1H" => out.weapon_grip = Some(WeaponGrip::OneHanded),
            "2H" => out.weapon_grip = Some(WeaponGrip::TwoHanded),

            other => out.subtypes.push(other.to_string()),
        }
    }

    // FaB's "Attack Action" cards carry both "Action" and "Attack" in types.
    // "Action" alone is a non-attack action (Aura, etc.). "Attack" alone on
    // a non-weapon shouldn't really happen; if it does we ignore (the Weapon
    // type will already be set for weapon entries).
    if out.has_action && out.has_attack {
        out.card_types.push(CardType::AttackAction);
    } else if out.has_action {
        out.card_types.push(CardType::Action);
    }

    // Default hero kind for a Hero with no explicit Young/Pit-Fighter marker.
    if out.card_types.contains(&CardType::Hero) && out.hero_kind.is_none() {
        out.hero_kind = Some(HeroKind::Adult);
    }

    out
}

/// Pull "X Specialization" entries out of `card_keywords`.
fn extract_specializations(keywords: &[String]) -> Vec<HeroMoniker> {
    keywords
        .iter()
        .filter_map(|kw| kw.strip_suffix(" Specialization"))
        .map(|m| HeroMoniker::new(m.trim()))
        .collect()
}

/// Extract `Essence of <terms>` grants from a hero's functional text.
///
/// FaB heroes carry essence abilities like "Essence of Earth, Ice, and
/// Lightning" — these augment the hero's effective supertypes for the
/// CR 1.1.3 subset rule. The canonical wording lives at the start of the
/// hero's `functional_text`, bolded with `**...**`. This parser handles the
/// bold form first; if not found, it falls back to the unbolded form.
///
/// Recognized term separators: comma, " and ", and the Oxford-comma
/// "X, Y, and Z" form. Terms that don't classify as a known Class or Talent
/// are silently dropped.
fn extract_essence_grants(text: &str) -> Vec<EssenceGrant> {
    let after_marker = text
        .find("**Essence of ")
        .map(|i| (i + "**Essence of ".len(), "**"))
        .or_else(|| {
            text.find("Essence of ")
                .map(|i| (i + "Essence of ".len(), "\n"))
        });
    let Some((start, end_marker)) = after_marker else {
        return Vec::new();
    };
    let rest = &text[start..];
    let end = rest.find(end_marker).unwrap_or(rest.len());
    let terms_raw = &rest[..end];
    parse_essence_terms(terms_raw)
}

fn parse_essence_terms(s: &str) -> Vec<EssenceGrant> {
    let mut grants = Vec::new();
    for chunk in s.split(',') {
        // Each chunk may itself contain " and " (e.g. "Light and Lightning"
        // when there's no comma), or start with "and " (Oxford form).
        let chunk = chunk.trim().trim_end_matches('.');
        let chunk = chunk.strip_prefix("and ").unwrap_or(chunk);
        for sub in chunk.split(" and ") {
            let term = sub.trim();
            if let Some(g) = classify_essence_term(term) {
                grants.push(g);
            }
        }
    }
    grants
}

fn classify_essence_term(term: &str) -> Option<EssenceGrant> {
    match term {
        "Draconic" => Some(EssenceGrant::Talent(Talent::Draconic)),
        "Earth" => Some(EssenceGrant::Talent(Talent::Earth)),
        "Elemental" => Some(EssenceGrant::Talent(Talent::Elemental)),
        "Ice" => Some(EssenceGrant::Talent(Talent::Ice)),
        "Light" => Some(EssenceGrant::Talent(Talent::Light)),
        "Lightning" => Some(EssenceGrant::Talent(Talent::Lightning)),
        "Shadow" => Some(EssenceGrant::Talent(Talent::Shadow)),
        "Wind" => Some(EssenceGrant::Talent(Talent::Wind)),
        // No printed FaB hero grants a class via essence (yet); add here
        // when one does.
        _ => None,
    }
}

/// Detect a hero-text-driven weapon zone count override. Defaults to 2 when
/// no override is present.
///
/// Recognized patterns (from the four printed Kayo variants and Bolfar):
/// - "You can't equip weapons." → 0
/// - "You have N weapon zone[s]." → N
/// - "You start the game with N weapon zone[s]." → N
fn detect_weapon_zone_count(text: &str) -> u8 {
    if text.contains("You can't equip weapons") {
        return 0;
    }
    // Look for a digit followed by " weapon zone". Iterate splits and inspect
    // the last whitespace-delimited token of each preceding chunk.
    let mut chunks = text.split(" weapon zone");
    let first = chunks.next();
    if chunks.next().is_some() {
        if let Some(prefix) = first {
            if let Some(last_word) = prefix.split_whitespace().next_back() {
                if let Ok(n) = last_word.parse::<u8>() {
                    return n;
                }
            }
        }
    }
    2
}

fn parse_legality_summary(j: &CardJson) -> LegalitySummary {
    fn map(
        banned: bool,
        suspended: bool,
        restricted: bool,
        ll_retired: bool,
        legal: bool,
    ) -> FormatStatus {
        if banned {
            FormatStatus::Banned
        } else if suspended {
            FormatStatus::Suspended
        } else if restricted {
            FormatStatus::Restricted
        } else if ll_retired {
            FormatStatus::LivingLegendRetired
        } else if legal {
            FormatStatus::Legal
        } else {
            FormatStatus::NotEligible
        }
    }

    LegalitySummary {
        cc: map(
            j.cc_banned,
            j.cc_suspended,
            false,
            j.cc_living_legend,
            j.cc_legal,
        ),
        blitz: map(
            j.blitz_banned,
            j.blitz_suspended,
            false,
            j.blitz_living_legend,
            j.blitz_legal,
        ),
        commoner: map(
            j.commoner_banned,
            j.commoner_suspended,
            false,
            false,
            j.commoner_legal,
        ),
        silver_age: map(j.silver_age_banned, false, false, false, j.silver_age_legal),
        living_legend: map(j.ll_banned, false, j.ll_restricted, false, j.ll_legal),
        upf: map(j.upf_banned, false, false, false, true),
    }
}

fn parse_edition(s: &str) -> Edition {
    match s {
        "A" => Edition::Alpha,
        "U" => Edition::Unlimited,
        // "N" (Normal/standard print run) and "F" (First edition) collapse
        // to First in our domain — we don't currently distinguish them in
        // legality logic.
        _ => Edition::First,
    }
}

fn parse_foiling(s: &str) -> Foiling {
    match s {
        "S" => Foiling::Standard,
        "R" => Foiling::RainbowFoil,
        "C" => Foiling::ColdFoil,
        "G" => Foiling::GoldFoil,
        _ => Foiling::Standard,
    }
}

fn parse_rarity(s: &str) -> Rarity {
    match s {
        "C" => Rarity::Common,
        "R" => Rarity::Rare,
        "S" => Rarity::SuperRare,
        "M" => Rarity::Majestic,
        "L" => Rarity::Legendary,
        "F" => Rarity::Fabled,
        "P" => Rarity::Promo,
        "T" => Rarity::Token,
        _ => Rarity::Common,
    }
}

// ---- conversion functions ----

pub fn convert_card(j: &CardJson) -> Card {
    let classified = classify_types(&j.types);

    let hero = if classified.card_types.contains(&CardType::Hero) {
        let text = j.functional_text.as_deref().unwrap_or("");
        Some(HeroFacts {
            kind: classified.hero_kind.unwrap_or(HeroKind::Adult),
            life: parse_u16(&j.health).unwrap_or(0),
            intellect: parse_u8(&j.intelligence),
            arcane: parse_u8(&j.arcane),
            essence_grants: extract_essence_grants(text),
            weapon_zone_count: detect_weapon_zone_count(text),
        })
    } else {
        None
    };

    Card {
        id: CardId::new(&j.unique_id),
        name: j.name.clone(),
        pitch: parse_u8(&j.pitch),
        cost: parse_u8(&j.cost),
        power: parse_u8(&j.power),
        defense: parse_u8(&j.defense),
        types: classified.card_types,
        subtypes: classified.subtypes,
        classes: classified.classes,
        talents: classified.talents,
        keywords: j.card_keywords.iter().map(Keyword::new).collect(),
        specializations: extract_specializations(&j.card_keywords),
        functional_text: j.functional_text.clone(),
        type_text: j.type_text.clone(),
        flavor_text: None,
        hero,
        weapon_grip: classified.weapon_grip,
        legality_summary: parse_legality_summary(j),
    }
}

pub fn convert_printing(
    j: &PrintingJson,
    card_id: &CardId,
    set_releases: &HashMap<String, NaiveDate>,
) -> Printing {
    Printing {
        id: PrintingId::new(&j.unique_id),
        card_id: card_id.clone(),
        set: SetCode::new(&j.set_id),
        set_release_date: set_releases
            .get(&j.set_id)
            .copied()
            .unwrap_or_else(placeholder_release_date),
        edition: parse_edition(&j.edition),
        foiling: parse_foiling(&j.foiling),
        // Treatment isn't directly encoded in the printing fields we sample;
        // the upstream uses art_variations/marvel-variant flags we don't
        // model yet. Default to Standard.
        treatment: Treatment::Standard,
        rarity: parse_rarity(&j.rarity),
        artist: j.artists.first().cloned(),
        collector_number: j.id.clone(),
        // Hot-link the CDN URL upstream provides; we never serve images
        // ourselves. Clients fetch from LSS's CDN directly.
        image_url: j.image_url.clone(),
    }
}

pub fn convert_banned_entry(j: &LegalityEntryJson) -> Option<BannedEntry> {
    let date = parse_iso_date(j.date_in_effect.as_deref()?)?;
    Some(BannedEntry {
        card_id: CardId::new(&j.card_unique_id),
        effective_date: date,
        status_active: j.status_active,
    })
}

pub fn convert_living_legend_entry(j: &LegalityEntryJson) -> Option<LivingLegendEntry> {
    let date = parse_iso_date(j.date_in_effect.as_deref()?)?;
    Some(LivingLegendEntry {
        card_id: CardId::new(&j.card_unique_id),
        effective_date: date,
    })
}

fn build_set_release_dates(sets: &[SetJson]) -> HashMap<String, NaiveDate> {
    let mut out = HashMap::with_capacity(sets.len());
    for set in sets {
        let earliest = set
            .printings
            .iter()
            .filter_map(|p| p.initial_release_date.as_deref())
            .filter_map(parse_iso_date)
            .min();
        if let Some(date) = earliest {
            out.insert(set.id.clone(), date);
        }
    }
    out
}

// ---- top-level builders ----

#[derive(Debug, Default)]
pub struct SyncOutput {
    pub catalog: Catalog,
    pub cc_banned: Vec<BannedEntry>,
    pub cc_living_legend: Vec<LivingLegendEntry>,
    pub card_count: usize,
    pub printing_count: usize,
}

/// Parse all upstream JSON content into a populated catalog and CC banlists.
/// Pure; performs no IO. Caller is responsible for fetching the JSON.
pub fn build_from_json(
    cards_json: &str,
    banned_cc_json: &str,
    living_legend_cc_json: &str,
    set_json: &str,
) -> Result<SyncOutput, SyncError> {
    let cards: Vec<CardJson> = serde_json::from_str(cards_json)?;
    let banned_entries: Vec<LegalityEntryJson> = serde_json::from_str(banned_cc_json)?;
    let ll_entries: Vec<LegalityEntryJson> = serde_json::from_str(living_legend_cc_json)?;
    let sets: Vec<SetJson> = serde_json::from_str(set_json)?;

    let set_releases = build_set_release_dates(&sets);

    let mut catalog = Catalog::new();
    let mut printing_count = 0;

    for j in &cards {
        let card = convert_card(j);
        let card_id = card.id.clone();
        for p in &j.printings {
            let printing = convert_printing(p, &card_id, &set_releases);
            catalog.insert_printing(printing);
            printing_count += 1;
        }
        catalog.insert_card(card);
    }

    let cc_banned = banned_entries
        .iter()
        .filter_map(convert_banned_entry)
        .collect();
    let cc_living_legend = ll_entries
        .iter()
        .filter_map(convert_living_legend_entry)
        .collect();

    Ok(SyncOutput {
        catalog,
        cc_banned,
        cc_living_legend,
        card_count: cards.len(),
        printing_count,
    })
}

// ---- HTTP fetch ----

#[derive(Debug, Clone)]
pub struct UpstreamSource {
    /// Git ref to fetch from. Use a tag or commit SHA in production; `main`
    /// is a fast-moving target.
    pub git_ref: String,
}

impl UpstreamSource {
    pub fn main() -> Self {
        Self {
            git_ref: "main".into(),
        }
    }

    pub fn at(git_ref: impl Into<String>) -> Self {
        Self {
            git_ref: git_ref.into(),
        }
    }

    fn url(&self, file: &str) -> String {
        format!("{UPSTREAM_REPO}/{}/json/english/{}", self.git_ref, file)
    }
}

pub async fn fetch_from_upstream(
    client: &reqwest::Client,
    source: &UpstreamSource,
) -> Result<SyncOutput, SyncError> {
    let raw = fetch_files(client, source).await?;
    build_from_json(&raw.cards, &raw.banned, &raw.ll, &raw.sets)
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, SyncError> {
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response.text().await?)
}

// ---- cache layer ----

/// On-disk cache config for sync data. Cache layout under `directory`
/// is one subdirectory per upstream git ref so switching refs (e.g. main →
/// pinned tag) doesn't read each other's data.
#[derive(Debug, Clone)]
pub struct SyncCache {
    pub directory: PathBuf,
    pub ttl: Duration,
}

const CACHE_FILES: &[&str] = &[
    "card.json",
    "banned-cc.json",
    "living-legend-cc.json",
    "set.json",
];

struct RawSyncFiles {
    cards: String,
    banned: String,
    ll: String,
    sets: String,
}

/// Load the catalog + CC banlists, preferring the cache when fresh.
///
/// Behavior:
/// - Cache present and fresh (mtime within `cache.ttl`): load from disk.
/// - Cache stale or missing: fetch from upstream, write cache, then parse.
/// - No cache passed: always fetch (no on-disk persistence).
///
/// Errors propagate from HTTP, IO, or JSON parsing. The cache is *not*
/// consulted as a fallback when fetch fails — by design we'd rather fail
/// loud at startup than serve stale data silently.
pub async fn load_or_fetch(
    client: &reqwest::Client,
    source: &UpstreamSource,
    cache: Option<&SyncCache>,
) -> Result<SyncOutput, SyncError> {
    if let Some(cache) = cache {
        let dir = cache_dir_for_ref(cache, source);
        if cache_is_fresh(&dir, cache.ttl)? {
            tracing::info!(directory = ?dir, "loading card data from cache");
            return load_from_cache(&dir);
        }
        tracing::info!(directory = ?dir, "cache miss or stale; fetching upstream");
    } else {
        tracing::info!("no cache configured; fetching upstream");
    }

    let raw = fetch_files(client, source).await?;

    if let Some(cache) = cache {
        let dir = cache_dir_for_ref(cache, source);
        if let Err(err) = write_to_cache(&dir, &raw) {
            tracing::warn!(?err, directory = ?dir, "failed to write sync cache; continuing");
        }
    }

    build_from_json(&raw.cards, &raw.banned, &raw.ll, &raw.sets)
}

async fn fetch_files(
    client: &reqwest::Client,
    source: &UpstreamSource,
) -> Result<RawSyncFiles, SyncError> {
    Ok(RawSyncFiles {
        cards: fetch_text(client, &source.url("card.json")).await?,
        banned: fetch_text(client, &source.url("banned-cc.json")).await?,
        ll: fetch_text(client, &source.url("living-legend-cc.json")).await?,
        sets: fetch_text(client, &source.url("set.json")).await?,
    })
}

fn cache_dir_for_ref(cache: &SyncCache, source: &UpstreamSource) -> PathBuf {
    cache.directory.join(&source.git_ref)
}

fn cache_is_fresh(dir: &Path, ttl: Duration) -> Result<bool, SyncError> {
    // Use card.json's mtime as the freshness probe; the four files are
    // written together atomically enough that they share the same age.
    let probe = dir.join("card.json");
    match std::fs::metadata(&probe) {
        Ok(meta) => {
            let modified = meta.modified()?;
            let age = SystemTime::now()
                .duration_since(modified)
                .unwrap_or(Duration::ZERO);
            Ok(age < ttl)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

fn load_from_cache(dir: &Path) -> Result<SyncOutput, SyncError> {
    let cards = std::fs::read_to_string(dir.join("card.json"))?;
    let banned = std::fs::read_to_string(dir.join("banned-cc.json"))?;
    let ll = std::fs::read_to_string(dir.join("living-legend-cc.json"))?;
    let sets = std::fs::read_to_string(dir.join("set.json"))?;
    build_from_json(&cards, &banned, &ll, &sets)
}

fn write_to_cache(dir: &Path, raw: &RawSyncFiles) -> Result<(), SyncError> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("card.json"), &raw.cards)?;
    std::fs::write(dir.join("banned-cc.json"), &raw.banned)?;
    std::fs::write(dir.join("living-legend-cc.json"), &raw.ll)?;
    std::fs::write(dir.join("set.json"), &raw.sets)?;
    tracing::debug!(directory = ?dir, files = ?CACHE_FILES, "wrote sync cache");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------- helper functions --------

    #[test]
    fn it_parses_iso_date_with_zulu_timezone() {
        let date = parse_iso_date("2024-03-25T00:00:00.000Z").unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 3, 25).unwrap());
    }

    #[test]
    fn it_returns_none_for_invalid_iso_date() {
        assert!(parse_iso_date("not a date").is_none());
        assert!(parse_iso_date("").is_none());
    }

    #[test]
    fn it_parses_empty_string_as_none_for_u8() {
        assert_eq!(parse_u8(""), None);
        assert_eq!(parse_u8("3"), Some(3));
        assert_eq!(parse_u8("not a number"), None);
    }

    #[test]
    fn it_parses_health_for_hero() {
        assert_eq!(parse_u16("40"), Some(40));
        assert_eq!(parse_u16(""), None);
    }

    // -------- classify_types --------

    #[test]
    fn it_classifies_attack_action_card() {
        let c = classify_types(&["Guardian".into(), "Action".into(), "Attack".into()]);
        assert_eq!(c.classes, vec![Class::Guardian]);
        assert!(c.card_types.contains(&CardType::AttackAction));
        assert!(!c.card_types.contains(&CardType::Action));
    }

    #[test]
    fn it_classifies_non_attack_action_card() {
        let c = classify_types(&["Illusionist".into(), "Action".into(), "Aura".into()]);
        assert!(c.card_types.contains(&CardType::Action));
        assert!(!c.card_types.contains(&CardType::AttackAction));
        assert_eq!(c.subtypes, vec!["Aura".to_string()]);
    }

    #[test]
    fn it_classifies_two_handed_weapon() {
        let c = classify_types(&[
            "Guardian".into(),
            "Weapon".into(),
            "Hammer".into(),
            "2H".into(),
        ]);
        assert!(c.card_types.contains(&CardType::Weapon));
        assert_eq!(c.weapon_grip, Some(WeaponGrip::TwoHanded));
        assert_eq!(c.subtypes, vec!["Hammer".to_string()]);
    }

    #[test]
    fn it_classifies_adult_hero() {
        let c = classify_types(&["Elemental".into(), "Guardian".into(), "Hero".into()]);
        assert!(c.card_types.contains(&CardType::Hero));
        assert_eq!(c.hero_kind, Some(HeroKind::Adult));
        assert_eq!(c.classes, vec![Class::Guardian]);
        assert_eq!(c.talents, vec![Talent::Elemental]);
    }

    #[test]
    fn it_classifies_young_hero() {
        let c = classify_types(&["Brute".into(), "Hero".into(), "Young".into()]);
        assert_eq!(c.hero_kind, Some(HeroKind::Young));
    }

    #[test]
    fn it_classifies_pit_fighter_hero() {
        let c = classify_types(&["Guardian".into(), "Hero".into(), "Pit-Fighter".into()]);
        assert_eq!(c.hero_kind, Some(HeroKind::PitFighter));
    }

    #[test]
    fn it_classifies_equipment_with_slot_subtype() {
        let c = classify_types(&["Earth".into(), "Equipment".into(), "Head".into()]);
        assert!(c.card_types.contains(&CardType::Equipment));
        assert_eq!(c.talents, vec![Talent::Earth]);
        assert_eq!(c.subtypes, vec!["Head".to_string()]);
    }

    #[test]
    fn it_preserves_unknown_type_strings_as_subtypes() {
        let c = classify_types(&["Action".into(), "FreshlyPrintedSubtype".into()]);
        assert!(c.subtypes.contains(&"FreshlyPrintedSubtype".to_string()));
    }

    // -------- extract_essence_grants --------

    #[test]
    fn it_extracts_three_essence_talents_from_oxford_comma_form() {
        let text = "**Essence of Earth, Ice, and Lightning**\n\nAt the start of your turn...";
        let grants = extract_essence_grants(text);
        assert_eq!(
            grants,
            vec![
                EssenceGrant::Talent(Talent::Earth),
                EssenceGrant::Talent(Talent::Ice),
                EssenceGrant::Talent(Talent::Lightning),
            ]
        );
    }

    #[test]
    fn it_extracts_single_essence_talent() {
        let text = "**Essence of Light**\n\nWhen you defend...";
        let grants = extract_essence_grants(text);
        assert_eq!(grants, vec![EssenceGrant::Talent(Talent::Light)]);
    }

    #[test]
    fn it_extracts_two_essence_talents_with_and() {
        let text = "**Essence of Light and Lightning**\n\nWhen you...";
        let grants = extract_essence_grants(text);
        assert_eq!(
            grants,
            vec![
                EssenceGrant::Talent(Talent::Light),
                EssenceGrant::Talent(Talent::Lightning),
            ]
        );
    }

    #[test]
    fn it_returns_empty_when_no_essence_marker_present() {
        assert!(extract_essence_grants("Some random hero text.").is_empty());
        assert!(extract_essence_grants("").is_empty());
    }

    #[test]
    fn it_silently_drops_unknown_essence_terms() {
        // "Fire" isn't a known FaB talent — should be dropped, not panic.
        let text = "**Essence of Earth and Fire**\n\nFoo";
        let grants = extract_essence_grants(text);
        assert_eq!(grants, vec![EssenceGrant::Talent(Talent::Earth)]);
    }

    // -------- detect_weapon_zone_count --------

    #[test]
    fn it_detects_one_weapon_zone_from_have_phrasing() {
        let text = "You have 1 weapon zone.\n\nAt the start of your turn...";
        assert_eq!(detect_weapon_zone_count(text), 1);
    }

    #[test]
    fn it_detects_one_weapon_zone_from_start_of_game_phrasing() {
        let text = "You start the game with 1 weapon zone.";
        assert_eq!(detect_weapon_zone_count(text), 1);
    }

    #[test]
    fn it_detects_zero_weapon_zones_when_cant_equip_weapons() {
        let text = "You can't equip weapons.\n\nOnce per turn...";
        assert_eq!(detect_weapon_zone_count(text), 0);
    }

    #[test]
    fn it_defaults_to_two_weapon_zones_for_normal_hero_text() {
        let text = "When you attack, do something.";
        assert_eq!(detect_weapon_zone_count(text), 2);
        assert_eq!(detect_weapon_zone_count(""), 2);
    }

    // -------- extract_specializations --------

    #[test]
    fn it_extracts_specialization_from_card_keywords() {
        let monikers = extract_specializations(&["Bravo Specialization".into(), "Crush".into()]);
        assert_eq!(monikers, vec![HeroMoniker::new("Bravo")]);
    }

    #[test]
    fn it_returns_empty_for_no_specialization_keyword() {
        assert!(extract_specializations(&["Crush".into()]).is_empty());
    }

    // -------- parse_legality_summary --------

    #[test]
    fn it_marks_banned_card_as_banned_in_summary() {
        let j = sample_card_json_banned_in_cc();
        let s = parse_legality_summary(&j);
        assert_eq!(s.cc, FormatStatus::Banned);
        assert_eq!(s.blitz, FormatStatus::Legal);
    }

    #[test]
    fn it_marks_living_legend_hero_as_retired_in_cc() {
        let mut j = sample_card_json_hero();
        j.cc_living_legend = true;
        let s = parse_legality_summary(&j);
        assert_eq!(s.cc, FormatStatus::LivingLegendRetired);
    }

    // -------- convert_card --------

    #[test]
    fn it_converts_an_attack_action_card() {
        let j = sample_card_json_attack_action();
        let card = convert_card(&j);
        assert_eq!(card.id.as_str(), "j8jjnw6NmpTpf9cWThfgg");
        assert_eq!(card.name, "Crippling Crush");
        assert_eq!(card.pitch, Some(1));
        assert_eq!(card.cost, Some(7));
        assert_eq!(card.power, Some(11));
        assert_eq!(card.defense, Some(3));
        assert_eq!(card.classes, vec![Class::Guardian]);
        assert!(card.types.contains(&CardType::AttackAction));
        assert_eq!(card.specializations, vec![HeroMoniker::new("Bravo")]);
        assert!(card.hero.is_none());
        assert!(card.weapon_grip.is_none());
    }

    #[test]
    fn it_converts_a_hero_card() {
        let j = sample_card_json_hero();
        let card = convert_card(&j);
        assert_eq!(card.name, "Bravo, Star of the Show");
        assert!(card.types.contains(&CardType::Hero));
        let hero = card.hero.unwrap();
        assert_eq!(hero.kind, HeroKind::Adult);
        assert_eq!(hero.life, 40);
        assert_eq!(hero.intellect, Some(4));
        assert_eq!(hero.weapon_zone_count, 2);
        // Bravo's text grants Essence of Earth, Ice, and Lightning.
        assert_eq!(
            hero.essence_grants,
            vec![
                EssenceGrant::Talent(Talent::Earth),
                EssenceGrant::Talent(Talent::Ice),
                EssenceGrant::Talent(Talent::Lightning),
            ]
        );
    }

    #[test]
    fn it_converts_a_one_zone_hero() {
        let mut j = empty_card_json("kayo_adult", "Kayo, Armed and Dangerous");
        j.health = "40".into();
        j.intelligence = "4".into();
        j.types = vec!["Brute".into(), "Hero".into()];
        j.functional_text = Some("You have 1 weapon zone.\n\nWhen Kayo...".into());
        j.cc_legal = true;
        let card = convert_card(&j);
        let hero = card.hero.unwrap();
        assert_eq!(hero.kind, HeroKind::Adult);
        assert_eq!(hero.weapon_zone_count, 1);
    }

    #[test]
    fn it_converts_a_zero_zone_hero() {
        let mut j = empty_card_json("bolfar", "Bolfar, Bear Hands");
        j.health = "20".into();
        j.intelligence = "4".into();
        j.types = vec!["Guardian".into(), "Hero".into(), "Pit-Fighter".into()];
        j.functional_text = Some("You can't equip weapons.\n\nOnce per turn...".into());
        let card = convert_card(&j);
        let hero = card.hero.unwrap();
        assert_eq!(hero.kind, HeroKind::PitFighter);
        assert_eq!(hero.weapon_zone_count, 0);
    }

    #[test]
    fn it_converts_a_two_handed_weapon() {
        let j = sample_card_json_2h_weapon();
        let card = convert_card(&j);
        assert!(card.types.contains(&CardType::Weapon));
        assert_eq!(card.weapon_grip, Some(WeaponGrip::TwoHanded));
        assert_eq!(card.power, Some(4));
    }

    // -------- convert_printing --------

    #[test]
    fn it_converts_a_printing_with_known_set_release_date() {
        let mut releases = HashMap::new();
        releases.insert("WTR".into(), NaiveDate::from_ymd_opt(2019, 10, 11).unwrap());
        let j = PrintingJson {
            unique_id: "p1".into(),
            set_id: "WTR".into(),
            id: "WTR001".into(),
            edition: "A".into(),
            foiling: "C".into(),
            rarity: "L".into(),
            artists: vec!["Some Artist".into()],
            image_url: None,
        };
        let printing = convert_printing(&j, &CardId::new("c1"), &releases);
        assert_eq!(printing.id.as_str(), "p1");
        assert_eq!(printing.card_id.as_str(), "c1");
        assert_eq!(printing.edition, Edition::Alpha);
        assert_eq!(printing.foiling, Foiling::ColdFoil);
        assert_eq!(printing.rarity, Rarity::Legendary);
        assert_eq!(
            printing.set_release_date,
            NaiveDate::from_ymd_opt(2019, 10, 11).unwrap()
        );
    }

    #[test]
    fn it_falls_back_to_placeholder_when_set_release_unknown() {
        let releases = HashMap::new();
        let j = PrintingJson {
            unique_id: "p1".into(),
            set_id: "UNKNOWN".into(),
            id: "X001".into(),
            edition: "N".into(),
            foiling: "S".into(),
            rarity: "C".into(),
            artists: vec![],
            image_url: None,
        };
        let printing = convert_printing(&j, &CardId::new("c1"), &releases);
        assert_eq!(printing.set_release_date, placeholder_release_date());
    }

    #[test]
    fn it_passes_image_url_through_to_printing() {
        let releases = HashMap::new();
        let url = "https://storage.googleapis.com/fabmaster/cardfaces/2024-MST/EN/MST131.png";
        let j = PrintingJson {
            unique_id: "p1".into(),
            set_id: "MST".into(),
            id: "MST131".into(),
            edition: "N".into(),
            foiling: "S".into(),
            rarity: "M".into(),
            artists: vec![],
            image_url: Some(url.into()),
        };
        let printing = convert_printing(&j, &CardId::new("c1"), &releases);
        assert_eq!(printing.image_url.as_deref(), Some(url));
    }

    #[test]
    fn it_leaves_image_url_none_when_upstream_omits_it() {
        let releases = HashMap::new();
        let j = PrintingJson {
            unique_id: "p1".into(),
            set_id: "WTR".into(),
            id: "WTR001".into(),
            edition: "F".into(),
            foiling: "S".into(),
            rarity: "C".into(),
            artists: vec![],
            image_url: None,
        };
        let printing = convert_printing(&j, &CardId::new("c1"), &releases);
        assert!(printing.image_url.is_none());
    }

    // -------- legality entry conversion --------

    #[test]
    fn it_converts_a_banned_entry() {
        let j = LegalityEntryJson {
            card_unique_id: "card_id".into(),
            status_active: true,
            date_announced: Some("2024-03-23T00:00:00.000Z".into()),
            date_in_effect: Some("2024-03-25T00:00:00.000Z".into()),
        };
        let entry = convert_banned_entry(&j).unwrap();
        assert_eq!(entry.card_id.as_str(), "card_id");
        assert!(entry.status_active);
        assert_eq!(
            entry.effective_date,
            NaiveDate::from_ymd_opt(2024, 3, 25).unwrap()
        );
    }

    #[test]
    fn it_drops_banned_entry_with_missing_effective_date() {
        let j = LegalityEntryJson {
            card_unique_id: "x".into(),
            status_active: true,
            date_announced: None,
            date_in_effect: None,
        };
        assert!(convert_banned_entry(&j).is_none());
    }

    // -------- end-to-end build_from_json --------

    #[test]
    fn it_builds_a_catalog_from_realistic_json() {
        let cards_json = realistic_cards_json();
        let banned_json = realistic_banned_cc_json();
        let ll_json = realistic_ll_cc_json();
        let sets_json = realistic_sets_json();

        let out = build_from_json(cards_json, banned_json, ll_json, sets_json).unwrap();
        assert_eq!(out.card_count, 3);
        assert!(out.printing_count >= 3);
        assert!(out.catalog.card(&CardId::new("hero_bravo")).is_some());
        assert!(out.catalog.card(&CardId::new("crippling_crush")).is_some());
        // Bravo is in the LL list.
        assert!(out
            .cc_living_legend
            .iter()
            .any(|e| e.card_id.as_str() == "hero_bravo"));
        // Crown of Seeds is in the banned list.
        assert!(out
            .cc_banned
            .iter()
            .any(|e| e.card_id.as_str() == "crown_of_seeds"));
    }

    // -------- cache layer --------

    #[test]
    fn it_treats_missing_cache_dir_as_not_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("does-not-exist");
        assert!(!cache_is_fresh(&dir, Duration::from_secs(60)).unwrap());
    }

    #[test]
    fn it_treats_empty_cache_dir_as_not_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        // Directory exists but contains no card.json.
        assert!(!cache_is_fresh(tmp.path(), Duration::from_secs(60)).unwrap());
    }

    #[test]
    fn it_treats_just_written_cache_as_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("card.json"), "[]").unwrap();
        assert!(cache_is_fresh(tmp.path(), Duration::from_secs(60)).unwrap());
    }

    #[test]
    fn it_writes_and_reads_back_cache_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = RawSyncFiles {
            cards: realistic_cards_json().to_string(),
            banned: realistic_banned_cc_json().to_string(),
            ll: realistic_ll_cc_json().to_string(),
            sets: realistic_sets_json().to_string(),
        };
        write_to_cache(tmp.path(), &raw).unwrap();
        // All four files are present
        for file in CACHE_FILES {
            assert!(tmp.path().join(file).exists(), "missing cache file: {file}");
        }
        let out = load_from_cache(tmp.path()).unwrap();
        assert_eq!(out.card_count, 3);
        assert!(out
            .cc_banned
            .iter()
            .any(|e| e.card_id.as_str() == "crown_of_seeds"));
    }

    #[test]
    fn it_partitions_cache_dir_by_git_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SyncCache {
            directory: tmp.path().to_path_buf(),
            ttl: Duration::from_secs(60),
        };
        let main = UpstreamSource::main();
        let pinned = UpstreamSource::at("v1.2.3");
        assert_ne!(
            cache_dir_for_ref(&cache, &main),
            cache_dir_for_ref(&cache, &pinned)
        );
    }

    #[tokio::test]
    async fn it_loads_from_cache_without_calling_upstream_on_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SyncCache {
            directory: tmp.path().to_path_buf(),
            ttl: Duration::from_secs(60),
        };
        let source = UpstreamSource::main();
        // Pre-seed the cache directory at the git-ref-resolved path.
        let dir = cache_dir_for_ref(&cache, &source);
        std::fs::create_dir_all(&dir).unwrap();
        let raw = RawSyncFiles {
            cards: realistic_cards_json().to_string(),
            banned: realistic_banned_cc_json().to_string(),
            ll: realistic_ll_cc_json().to_string(),
            sets: realistic_sets_json().to_string(),
        };
        write_to_cache(&dir, &raw).unwrap();

        // Use a client pointing at an unreachable address. With the cache
        // pre-seeded and fresh, load_or_fetch should never await the
        // client.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1))
            .build()
            .unwrap();
        let out = load_or_fetch(&client, &source, Some(&cache)).await.unwrap();
        assert_eq!(out.card_count, 3);
    }

    // -------- smoke test against real upstream clone --------

    /// Manual smoke test. Run with:
    ///
    /// ```text
    /// FAB_CUBE_DATA_DIR=/path/to/fab-cards-clone \
    ///   cargo test --lib it_parses_full_upstream -- --ignored --nocapture
    /// ```
    ///
    /// Verifies the parser handles every card in the real upstream without
    /// panicking, and that landmark counts look sensible. Not run in CI;
    /// requires a local clone of `the-fab-cube/flesh-and-blood-cards`.
    #[test]
    #[ignore = "requires FAB_CUBE_DATA_DIR pointing to a local repo clone"]
    fn it_parses_full_upstream_data_from_disk() {
        let dir = std::env::var("FAB_CUBE_DATA_DIR").expect(
            "FAB_CUBE_DATA_DIR must point to a clone of the-fab-cube/flesh-and-blood-cards",
        );
        let cards = std::fs::read_to_string(format!("{dir}/json/english/card.json")).unwrap();
        let banned = std::fs::read_to_string(format!("{dir}/json/english/banned-cc.json")).unwrap();
        let ll =
            std::fs::read_to_string(format!("{dir}/json/english/living-legend-cc.json")).unwrap();
        let sets = std::fs::read_to_string(format!("{dir}/json/english/set.json")).unwrap();

        let out = build_from_json(&cards, &banned, &ll, &sets)
            .expect("upstream JSON should parse cleanly");

        eprintln!(
            "parsed {} cards, {} printings, {} CC banned entries, {} CC LL retired",
            out.card_count,
            out.printing_count,
            out.cc_banned.len(),
            out.cc_living_legend.len()
        );

        assert!(
            out.card_count > 1000,
            "expected >1000 cards, got {}",
            out.card_count
        );
        assert!(out.printing_count > out.card_count);
        assert!(!out.cc_living_legend.is_empty());
        assert!(!out.cc_banned.is_empty());
    }

    // -------- fixtures --------

    fn empty_card_json(id: &str, name: &str) -> CardJson {
        CardJson {
            unique_id: id.into(),
            name: name.into(),
            color: String::new(),
            pitch: String::new(),
            cost: String::new(),
            power: String::new(),
            defense: String::new(),
            health: String::new(),
            intelligence: String::new(),
            arcane: String::new(),
            types: Vec::new(),
            card_keywords: Vec::new(),
            functional_text: None,
            type_text: None,
            blitz_legal: false,
            cc_legal: false,
            commoner_legal: false,
            ll_legal: false,
            silver_age_legal: false,
            blitz_living_legend: false,
            cc_living_legend: false,
            blitz_banned: false,
            cc_banned: false,
            commoner_banned: false,
            ll_banned: false,
            silver_age_banned: false,
            upf_banned: false,
            blitz_suspended: false,
            cc_suspended: false,
            commoner_suspended: false,
            ll_restricted: false,
            printings: Vec::new(),
        }
    }

    fn sample_card_json_attack_action() -> CardJson {
        let mut c = empty_card_json("j8jjnw6NmpTpf9cWThfgg", "Crippling Crush");
        c.color = "Red".into();
        c.pitch = "1".into();
        c.cost = "7".into();
        c.power = "11".into();
        c.defense = "3".into();
        c.types = vec!["Guardian".into(), "Action".into(), "Attack".into()];
        c.card_keywords = vec!["Bravo Specialization".into(), "Crush".into()];
        c.cc_legal = true;
        c.blitz_legal = true;
        c.ll_legal = true;
        c
    }

    fn sample_card_json_hero() -> CardJson {
        let mut c = empty_card_json("hero_bravo", "Bravo, Star of the Show");
        c.health = "40".into();
        c.intelligence = "4".into();
        c.types = vec!["Elemental".into(), "Guardian".into(), "Hero".into()];
        c.functional_text =
            Some("**Essence of Earth, Ice, and Lightning**\n\nAt the start of your turn...".into());
        c.cc_legal = true;
        c.ll_legal = true;
        c
    }

    fn sample_card_json_2h_weapon() -> CardJson {
        let mut c = empty_card_json("weapon_anothos", "Anothos");
        c.power = "4".into();
        c.types = vec![
            "Guardian".into(),
            "Weapon".into(),
            "Hammer".into(),
            "2H".into(),
        ];
        c.cc_legal = true;
        c.blitz_legal = true;
        c
    }

    fn sample_card_json_banned_in_cc() -> CardJson {
        let mut c = empty_card_json("crown_of_seeds", "Crown of Seeds");
        c.defense = "0".into();
        c.types = vec!["Earth".into(), "Equipment".into(), "Head".into()];
        c.cc_banned = true;
        c.blitz_legal = true;
        c
    }

    fn realistic_cards_json() -> &'static str {
        r#"[
            {
                "unique_id": "hero_bravo",
                "name": "Bravo, Star of the Show",
                "health": "40",
                "intelligence": "4",
                "types": ["Elemental", "Guardian", "Hero"],
                "card_keywords": [],
                "cc_legal": true,
                "cc_living_legend": true,
                "ll_legal": true,
                "printings": [
                    {
                        "unique_id": "bravo_p1",
                        "set_id": "EVR",
                        "id": "EVR017",
                        "edition": "F",
                        "foiling": "S",
                        "rarity": "P",
                        "artists": ["Artist"]
                    }
                ]
            },
            {
                "unique_id": "crippling_crush",
                "name": "Crippling Crush",
                "pitch": "1",
                "cost": "7",
                "power": "11",
                "defense": "3",
                "types": ["Guardian", "Action", "Attack"],
                "card_keywords": ["Bravo Specialization", "Crush"],
                "cc_legal": true,
                "blitz_legal": true,
                "ll_legal": true,
                "printings": [
                    {
                        "unique_id": "cc_p1",
                        "set_id": "WTR",
                        "id": "WTR036",
                        "edition": "F",
                        "foiling": "S",
                        "rarity": "M",
                        "artists": ["Artist"]
                    }
                ]
            },
            {
                "unique_id": "crown_of_seeds",
                "name": "Crown of Seeds",
                "defense": "0",
                "types": ["Earth", "Equipment", "Head"],
                "card_keywords": [],
                "cc_banned": true,
                "blitz_legal": true,
                "printings": [
                    {
                        "unique_id": "cos_p1",
                        "set_id": "ELE",
                        "id": "ELE115",
                        "edition": "F",
                        "foiling": "S",
                        "rarity": "M",
                        "artists": ["Artist"]
                    }
                ]
            }
        ]"#
    }

    fn realistic_banned_cc_json() -> &'static str {
        r#"[
            {
                "unique_id": "ban_entry_1",
                "card_unique_id": "crown_of_seeds",
                "status_active": true,
                "date_announced": "2024-03-23T00:00:00.000Z",
                "date_in_effect": "2024-03-25T00:00:00.000Z",
                "legality_article": "https://example.com/ban1"
            }
        ]"#
    }

    fn realistic_ll_cc_json() -> &'static str {
        r#"[
            {
                "unique_id": "ll_entry_1",
                "card_unique_id": "hero_bravo",
                "status_active": true,
                "date_announced": "2022-06-24T00:00:00.000Z",
                "date_in_effect": "2022-06-24T00:00:00.000Z",
                "legality_article": "https://example.com/ll1"
            }
        ]"#
    }

    fn realistic_sets_json() -> &'static str {
        r#"[
            {
                "id": "WTR",
                "name": "Welcome to Rathe",
                "printings": [
                    { "initial_release_date": "2019-10-11T00:00:00.000Z" }
                ]
            },
            {
                "id": "EVR",
                "name": "Everfest",
                "printings": [
                    { "initial_release_date": "2022-04-08T00:00:00.000Z" }
                ]
            },
            {
                "id": "ELE",
                "name": "Tales of Aria",
                "printings": [
                    { "initial_release_date": "2021-09-24T00:00:00.000Z" }
                ]
            }
        ]"#
    }
}
