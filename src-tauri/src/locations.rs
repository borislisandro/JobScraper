//! Where a job is, as a country, decided once when the job is stored.
//!
//! A board writes an office, not a country — "Lisbon", "Gratkorn", "Taichung - Fab 16 Taiwan" —
//! and the same office name belongs to several countries: there are San Joses in the United
//! States, Costa Rica and the Philippines, and Cambridge is in both England and Massachusetts. A
//! list of city keywords cannot separate those; matching them as text at query time only spreads
//! the ambiguity across every search. So each listing is resolved here, once, to the country codes
//! it actually names, and the Jobs page filters on the codes.
//!
//! Two rules do the disambiguating, in this order:
//!   1. the board's own words win — "Cambridge, United Kingdom" is GB, never US;
//!   2. otherwise the most populous namesake wins, and it wins alone. Keeping the runners-up as
//!      well filed 454 Santa Clara jobs under Cuba and 381 San Jose jobs under Costa Rica, which is
//!      the opposite of a filter: a country you pick has to return that country's jobs. A listing
//!      still carries several countries when it genuinely names several places.
//!
//! City data © GeoNames, CC BY 4.0 (see scripts/build-city-index.mjs). The files below are
//! generated and checked in; nothing is fetched at runtime.
use std::collections::HashMap;
use std::sync::OnceLock;

/// Bumped whenever the rules below change their answers, so every stored listing is worked out
/// again on the next launch. Without it a job keeps the country an older, wronger version of this
/// file gave it: the fixes that emptied Cuba of Californian jobs would never have reached the rows
/// already in the database.
pub const RESOLVER_VERSION: &str = "2";
static CITIES: &str = include_str!("../data/cities.tsv");
static COUNTRY_NAMES: &str = include_str!("../data/countries.tsv");
static FOLDS: &str = include_str!("../data/fold.tsv");
/// Longest city name worth looking for, in words. "Sophia Antipolis" needs two, and the longest
/// real entries ("Newcastle upon Tyne") need three; beyond that a window is a sentence.
const MAX_CITY_WORDS: usize = 4;
/// Words that appear where a place would and are not one. A board writes "Remote", "Hybrid", or
/// names the building; matching those against a village of the same name would put jobs in
/// countries nobody mentioned.
const NOT_A_PLACE: &[&str] = &[
    "remote",
    "hybrid",
    "onsite",
    "on site",
    "office",
    "offices",
    "home",
    "homeoffice",
    "headquarters",
    "hq",
    "campus",
    "site",
    "sites",
    "plant",
    "fab",
    "lab",
    "labs",
    "various",
    "multiple",
    "location",
    "locations",
    "other",
    "others",
    "all",
    "any",
    "area",
    "region",
    "metro",
    "city",
    "country",
    "worldwide",
    "global",
    "anywhere",
    "flexible",
    "more",
    "and",
    "the",
    "of",
];
/// A US board writes the state, not the country: "Austin, TX", "CT Simsbury". These are read only
/// as a hint that settles which homonym is meant — never as an answer on their own — and the ones
/// that are ordinary words when lower-cased (IN, OR, OK, ME, HI, LA, DE, CO, ID, MS) are left out,
/// because a hint is not worth a wrong country.
const US_STATES: &[&str] = &[
    "al",
    "ak",
    "az",
    "ar",
    "ct",
    "fl",
    "ga",
    "il",
    "ks",
    "ky",
    "md",
    "mn",
    "mo",
    "mt",
    "nc",
    "nd",
    "nh",
    "nj",
    "nm",
    "ny",
    "nv",
    "oh",
    "pa",
    "ri",
    "sc",
    "sd",
    "tn",
    "tx",
    "ut",
    "vt",
    "wa",
    "wi",
    "wv",
    "wy",
    "ca",
    "mi",
    "va",
    // Written out, which is how "Austin, Texas" and "Santa Clara, California" arrive. Georgia is
    // deliberately here as well as being a country: the segment then names both, and whichever the
    // city belongs to wins, so Atlanta stays American and Tbilisi stays Georgian.
    "alabama",
    "alaska",
    "arizona",
    "arkansas",
    "california",
    "colorado",
    "connecticut",
    "delaware",
    "florida",
    "georgia",
    "hawaii",
    "idaho",
    "illinois",
    "indiana",
    "iowa",
    "kansas",
    "kentucky",
    "louisiana",
    "maine",
    "maryland",
    "massachusetts",
    "michigan",
    "minnesota",
    "mississippi",
    "missouri",
    "montana",
    "nebraska",
    "nevada",
    "new hampshire",
    "new jersey",
    "new mexico",
    "new york",
    "north carolina",
    "north dakota",
    "ohio",
    "oklahoma",
    "oregon",
    "pennsylvania",
    "rhode island",
    "south carolina",
    "south dakota",
    "tennessee",
    "texas",
    "utah",
    "vermont",
    "virginia",
    "washington",
    "west virginia",
    "wisconsin",
    "wyoming",
];

fn table(source: &'static str) -> HashMap<&'static str, &'static str> {
    source
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect()
}
fn cities() -> &'static HashMap<&'static str, &'static str> {
    static INDEX: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    INDEX.get_or_init(|| table(CITIES))
}
fn country_names() -> &'static HashMap<&'static str, &'static str> {
    static INDEX: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    INDEX.get_or_init(|| table(COUNTRY_NAMES))
}
fn folds() -> &'static HashMap<char, &'static str> {
    static INDEX: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    INDEX.get_or_init(|| {
        FOLDS
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter_map(|(from, to)| Some((from.chars().next()?, to)))
            .collect()
    })
}
/// Accents off, case down, punctuation to spaces — the same shape the index was written in. The
/// accent table is generated by the script that writes the index, from the same code, so the two
/// halves cannot drift; anything else non-ASCII becomes a separator rather than a false letter.
pub fn fold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if let Some(plain) =
            folds().get(&character.to_lowercase().next().unwrap_or(character))
        {
            out.push_str(plain);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// The office in a Workday job path: "/job/Taichung---Fab-16-Taiwan/Staff-Engineer_JR1". Mirrors
/// pathCity() in sidecar/worker.mjs, and exists because the listings that say only "4 Locations"
/// still carry their primary office right there in the URL.
fn path_place(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let raw = parts
        .by_ref()
        .find(|part| *part == "job")
        .and(parts.next())?;
    let place = fold(raw);
    // A requisition sits in that slot on some tenants; an office is words, even when the words
    // carry a fab number.
    let has_word = place
        .split(' ')
        .any(|word| word.len() >= 3 && word.chars().all(|c| c.is_ascii_alphabetic()));
    has_word.then_some(place)
}
/// Every country a listing names, most confident first, de-duplicated. Empty means the board said
/// nothing this index recognises — a count ("4 Locations"), a building, or a city nobody mapped.
pub fn resolve(location: Option<&str>, url: Option<&str>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    if let Some(text) = location {
        for segment in segments(text) {
            resolve_segment(&segment, &mut found);
        }
    }
    // Only when the words gave nothing: a URL is weaker evidence than a published location, but it
    // is far better than filing the job under no country at all.
    if found.is_empty() {
        if let Some(place) = url.and_then(path_place) {
            resolve_segment(&[place], &mut found);
        }
    }
    found
}
/// Split into groups that belong together. A semicolon or a slash separates places; a comma inside
/// one group usually qualifies it ("Cambridge, United Kingdom"), which is exactly the pairing the
/// country rule needs, so commas split parts within a group rather than the group itself.
fn segments(text: &str) -> Vec<Vec<String>> {
    // "(+2 more)" and "(Oakhill, Office)" describe the listing, not a place.
    let mut cleaned = String::with_capacity(text.len());
    let mut depth = 0usize;
    for character in text.chars() {
        match character {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => cleaned.push(character),
            _ => {}
        }
    }
    cleaned
        .split([';', '|', '/', '\u{00b7}', '\u{2022}'])
        .map(|segment| {
            segment
                .split([',', '-', '\u{2013}'])
                .map(fold)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|parts: &Vec<String>| !parts.is_empty())
        .collect()
}
fn resolve_segment(parts: &[String], found: &mut Vec<String>) {
    // What the segment says outright, wherever inside it that is: boards write "Malaysia Penang
    // MCHP" and "United Kingdom Whiteley" as one unpunctuated run, so the country has to be looked
    // for in the words rather than only in a comma-separated part of its own.
    let mut named: Vec<&'static str> = Vec::new();
    // Names that are a country AND a US state: Georgia is both, and "New Jersey" contains Jersey.
    let mut also_a_state: Vec<&'static str> = Vec::new();
    let mut us_named_outright = false;
    for part in parts {
        for window in windows(part) {
            let country = country_names().get(window.as_str()).copied();
            let state = US_STATES.contains(&window.as_str());
            if country.is_none() && !state {
                continue;
            }
            if state && !named.contains(&"US") {
                named.push("US");
            }
            if let Some(code) = country {
                if state {
                    also_a_state.push(code);
                } else if code == "US" {
                    us_named_outright = true;
                }
                if !named.contains(&code) {
                    named.push(code);
                }
            } else {
                us_named_outright = true;
            }
            // Longest match wins for this part and nothing shorter is looked at, so "New Jersey" is
            // read as the state and never as the island of Jersey inside it.
            break;
        }
    }
    // "Georgia - Remote, United States of America" is Atlanta's Georgia. The country only yields
    // when the listing named the United States by something other than that same word — otherwise
    // "Tbilisi, Georgia" would be dragged to America too.
    if us_named_outright {
        named.retain(|code| *code == "US" || !also_a_state.contains(code));
    }
    let mut hit = false;
    for part in parts {
        if country_names().contains_key(part.as_str()) {
            continue;
        }
        for code in city_countries(part, &named) {
            push(found, code);
            hit = true;
        }
    }
    // A segment that named its country and nothing else recognisable still means that country.
    if !hit {
        for code in named {
            push(found, code.to_string());
        }
    }
}
/// Every word run in a part, longest first, so the longest name that matches anything wins:
/// "united kingdom whiteley" has to be read as United Kingdom, not as three separate words.
fn windows(part: &str) -> Vec<String> {
    let words: Vec<&str> = part.split(' ').filter(|word| !word.is_empty()).collect();
    let mut out = Vec::new();
    for width in (1..=MAX_CITY_WORDS.min(words.len())).rev() {
        for start in 0..=words.len() - width {
            out.push(words[start..start + width].join(" "));
        }
    }
    out
}
/// The countries of the longest place name inside one part. One answer when the segment named a
/// country, otherwise the leading homonym plus any rival big enough to be plausibly meant.
fn city_countries(part: &str, named: &[&'static str]) -> Vec<String> {
    for candidate in windows(part) {
        {
            if candidate.len() < 3
                || NOT_A_PLACE.contains(&candidate.as_str())
                || candidate.chars().all(|c| c.is_ascii_digit())
            {
                continue;
            }
            // A word that names a country is read as that country, not as the village elsewhere
            // that happens to share the name: "India Bangalore" is India, not Inđija in Serbia.
            if country_names().contains_key(candidate.as_str()) {
                continue;
            }
            let Some(row) = cities().get(candidate.as_str()) else {
                continue;
            };
            // Rows are written most populous first.
            let places: Vec<(&str, u64)> = row
                .split(',')
                .filter_map(|entry| entry.split_once(':'))
                .map(|(code, population)| (code, population.parse().unwrap_or(0)))
                .collect();
            let Some(&(leader, _)) = places.first() else {
                continue;
            };
            // The board's own word settles it outright. When it named a country and this place is
            // not in it, the match is a coincidence — keep looking, and let the named country stand
            // if nothing better turns up. Emitting the coincidence instead is how "India Bangalore"
            // used to come out as Serbia.
            if !named.is_empty() {
                if let Some(named_here) = places
                    .iter()
                    .find(|(code, _)| named.contains(code))
                    .map(|(code, _)| code.to_string())
                {
                    return vec![named_here];
                }
                continue;
            }
            // One place, one country. Cuba's Santa Clara outgrew California's, so this is
            // sometimes the wrong one — but a wrong country is at least a country nobody else's
            // filter has to wade through, and naming the state or the country in the listing
            // overrules it above.
            return vec![leader.to_string()];
        }
    }
    Vec::new()
}
fn push(found: &mut Vec<String>, code: String) {
    if !found.contains(&code) {
        found.push(code);
    }
}
/// How the answer is stored: ",PT,ES," so a query can test one code with a plain substring match,
/// and an empty string means "resolved, nothing found" as opposed to NULL's "not looked at yet".
pub fn stored(codes: &[String]) -> String {
    if codes.is_empty() {
        String::new()
    } else {
        format!(",{},", codes.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn codes(location: &str) -> Vec<String> {
        resolve(Some(location), None)
    }
    #[test]
    fn the_board_s_own_country_beats_the_bigger_homonym() {
        // The case a keyword list cannot do: same city name, two countries, decided by the words
        // next to it.
        assert_eq!(codes("San Jose, Costa Rica"), vec!["CR"]);
        assert_eq!(codes("San Jose"), vec!["US"]);
        assert_eq!(codes("Cambridge, United Kingdom"), vec!["GB"]);
        // Nothing names a country, so the biggest Cambridge wins and the others are not guessed at.
        assert_eq!(codes("Cambridge"), vec!["GB"]);
        assert_eq!(codes("Valencia, Venezuela"), vec!["VE"]);
        assert_eq!(codes("Valencia"), vec!["VE"]);
    }
    #[test]
    fn a_city_stands_in_for_its_country_and_accents_do_not_matter() {
        assert_eq!(codes("Lisbon"), vec!["PT"]);
        assert_eq!(codes("Lisboa"), vec!["PT"]);
        assert_eq!(codes("Porto"), vec!["PT"]);
        assert_eq!(codes("München"), vec!["DE"]);
        assert_eq!(codes("Munich"), vec!["DE"]);
        assert_eq!(codes("Kuala Lumpur"), vec!["MY"]);
        assert_eq!(codes("Bengaluru"), vec!["IN"]);
        // Small towns are why the whole index is bundled rather than a hand-written list.
        assert_eq!(codes("Gratkorn"), vec!["AT"]);
        assert_eq!(codes("Crolles"), vec!["FR"]);
        assert_eq!(codes("Nijmegen"), vec!["NL"]);
    }
    #[test]
    fn every_place_in_a_multi_location_listing_counts() {
        assert_eq!(codes("Sibiu, Caen"), vec!["RO", "FR"]);
        assert_eq!(codes("Austin (Oakhill, Office)"), vec!["US"]);
        assert_eq!(codes("Taichung - Fab 16 Taiwan"), vec!["TW"]);
        assert_eq!(codes("Sibiu (+1 more)"), vec!["RO"]);
        assert_eq!(codes("Remote - Germany"), vec!["DE"]);
    }
    #[test]
    fn a_count_is_not_a_place_but_its_url_still_is() {
        assert!(codes("4 Locations").is_empty());
        assert!(codes("Remote").is_empty());
        assert!(codes("").is_empty());
        // Exactly the Micron rows: the listing says only how many, the link says where.
        assert_eq!(
            resolve(
                Some("2 Locations"),
                Some("https://micron.wd1.myworkdayjobs.com/External/job/Taichung---Fab-16-Taiwan/Staff-Engineer_JR108702")
            ),
            vec!["TW"]
        );
        assert_eq!(
            resolve(
                Some("4 Locations"),
                Some("https://micron.wd1.myworkdayjobs.com/External/job/Richardson-TX/Principal-Design-Engineer_JR109173")
            ),
            vec!["US"]
        );
        // A published location is never overruled by the URL.
        assert_eq!(
            resolve(Some("Lisbon"), Some("https://x.test/job/Austin/y_JR1")),
            vec!["PT"]
        );
    }
    // Both of these came out of a dry run over 14,529 real listings, and both were silent: the
    // jobs were filed under a country nobody would ever look for them in.
    #[test]
    fn a_word_that_names_a_country_is_not_read_as_a_village_of_that_name() {
        // GeoNames has a Serbian town whose ascii name is exactly "India" (24k people), and it was
        // the only candidate, so "India Bangalore" was filed under Serbia.
        assert_eq!(codes("India Bangalore"), vec!["IN"]);
        assert_eq!(codes("India Chennai"), vec!["IN"]);
        assert_eq!(codes("Mexico City"), vec!["MX"]);
        assert_eq!(codes("Singapore"), vec!["SG"]);
    }
    #[test]
    fn the_bigger_namesake_does_not_swallow_the_one_the_board_meant() {
        // Cuba's Santa Clara is twice the size of California's, so population alone sent 897 chip
        // jobs to Cuba. The state settles it outright, and where nothing does, both are kept.
        assert_eq!(codes("US CA Santa Clara"), vec!["US"]);
        assert_eq!(codes("Santa Clara, California"), vec!["US"]);
        // Bare, with nothing to go on, it is Cuba's — and only Cuba's. Under the old rule this
        // carried the United States as well, so every Cuba filter returned Californian jobs.
        assert_eq!(codes("Santa Clara"), vec!["CU"]);
    }
    // A country name hiding inside a place name, found by auditing 14,529 real listings: every one
    // of these put American jobs in someone else's country.
    #[test]
    fn a_country_name_inside_a_bigger_place_name_is_not_that_country() {
        // Jersey is a country; New Jersey is not in it.
        assert_eq!(
            codes("Holmdel, New Jersey, United States of America"),
            vec!["US"]
        );
        // Mexico likewise.
        assert_eq!(codes("Albuquerque, New Mexico"), vec!["US"]);
        // Georgia is both a country and a state, so the listing decides which. Only a mention of
        // the United States by some other word settles it — the word itself never argues for both.
        assert_eq!(
            codes("Georgia - Remote, United States of America"),
            vec!["US"]
        );
        assert_eq!(codes("Atlanta, Georgia"), vec!["US"]);
        assert_eq!(codes("Tbilisi, Georgia"), vec!["GE"]);
    }
    // Every country a listing is filed under has to be one a person would find it in. Under the
    // earlier rule these each carried a second, wrong country: 454 Santa Clara jobs answered to
    // Cuba, 381 San Joses to Costa Rica, 331 Hyderabads to Pakistan.
    #[test]
    fn one_place_is_filed_under_one_country() {
        for location in [
            "US CA Santa Clara",
            "Santa Clara, California, United States of America",
            "San Jose, California",
            "Hyderabad, India",
            "IND Hyderabad 115 IT Park Area",
            "Cambridge, United Kingdom",
            "Vancouver, Canada",
        ] {
            assert_eq!(
                codes(location).len(),
                1,
                "{location} -> {:?}",
                codes(location)
            );
        }
        assert_eq!(codes("US CA Santa Clara"), vec!["US"]);
        assert_eq!(codes("IND Hyderabad 115 IT Park Area"), vec!["IN"]);
        // Several places named outright still mean several countries; that is not ambiguity.
        assert_eq!(codes("Krakow, Poland; Oeiras, Portugal"), vec!["PL", "PT"]);
    }
    #[test]
    fn a_country_written_inside_a_run_of_words_still_counts() {
        assert_eq!(codes("Malaysia Penang MCHP"), vec!["MY"]);
        assert_eq!(codes("United Kingdom Whiteley"), vec!["GB"]);
        assert_eq!(codes("Netherlands Remote"), vec!["NL"]);
        assert_eq!(codes("US Remote"), vec!["US"]);
        // Several offices in one string, each resolved on its own.
        assert_eq!(codes("Krakow, Poland; Oeiras, Portugal"), vec!["PL", "PT"]);
    }
    #[test]
    fn storage_shape_supports_a_substring_test_per_code() {
        assert_eq!(stored(&["PT".into(), "ES".into()]), ",PT,ES,");
        assert_eq!(stored(&[]), "");
        assert!(stored(&["PT".into()]).contains(",PT,"));
    }
}
