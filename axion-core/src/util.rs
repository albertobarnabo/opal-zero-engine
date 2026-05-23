/// Derive a short, stable, filename-safe slug from arbitrary text.
///
/// Lowercases all words, strips stop-words (union of the planner and persistence
/// lists so both call sites get identical behaviour), discards tokens that are
/// one character or shorter, takes the first `max_words` significant tokens, and
/// joins them with underscores.
///
/// Returns a UUID-derived fallback when `text` contains no significant words
/// (e.g. emoji-only or all-stop-word input).  Pass the fallback string you want
/// for the "empty" case via the `empty_fallback` parameter (`"mission"` for
/// persistence, a UUID prefix for the planner).
pub fn slugify(text: &str, max_words: usize) -> String {
    slugify_with_fallback(text, max_words, None)
}

/// Like [`slugify`] but uses a random UUID prefix as the empty fallback instead
/// of `"mission"`.  Used by the planner where a deterministic fallback could
/// silently collide across tasks.
pub fn slugify_unique_fallback(text: &str, max_words: usize) -> String {
    let fallback = uuid::Uuid::new_v4().to_string()[..8].to_string();
    slugify_with_fallback(text, max_words, Some(fallback))
}

fn slugify_with_fallback(text: &str, max_words: usize, fallback: Option<String>) -> String {
    // Union of the stop-word lists that previously lived in planner/mod.rs and
    // persistence/mod.rs.  Extra words from the planner list: "return",
    // "exactly", "using", "use", "report", "fact".
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "in", "on", "at", "to", "for", "of", "and", "or",
        "is", "are", "be", "this", "that", "it", "with", "from", "by",
        "return", "exactly", "using", "use", "report", "fact",
    ];

    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.len() > 1)
        .take(max_words)
        .collect();

    if words.is_empty() {
        return fallback.unwrap_or_else(|| "mission".to_string());
    }
    words.join("_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_empty_string_returns_mission() {
        assert_eq!(slugify("", 5), "mission");
    }

    #[test]
    fn slugify_all_stop_words_returns_mission() {
        assert_eq!(slugify("a the is an in on at to for of", 5), "mission");
    }

    #[test]
    fn slugify_emoji_only_returns_mission() {
        assert_eq!(slugify("🚀🌍✨", 5), "mission");
    }

    #[test]
    fn slugify_max_words_respected() {
        let result = slugify("find cheap flights from london to rome paris berlin", 3);
        let parts: Vec<&str> = result.split('_').collect();
        assert!(parts.len() <= 3, "Got {} words: {:?}", parts.len(), parts);
    }

    #[test]
    fn slugify_very_long_input_capped() {
        let long = "word1 word2 word3 word4 word5 word6 word7 word8 word9 word10";
        let result = slugify(long, 5);
        let parts: Vec<&str> = result.split('_').collect();
        assert_eq!(parts.len(), 5);
    }

    #[test]
    fn slugify_lowercases_all_words() {
        let result = slugify("FIND FLIGHTS ROME", 5);
        assert_eq!(result, result.to_lowercase());
    }

    #[test]
    fn slugify_filters_stop_words_from_output() {
        let result = slugify("find the best hotels in rome", 5);
        let parts: Vec<&str> = result.split('_').collect();
        assert!(!parts.contains(&"the"), "Stop word 'the' should be removed");
        assert!(!parts.contains(&"in"), "Stop word 'in' should be removed");
    }

    #[test]
    fn slugify_single_significant_word() {
        let result = slugify("rome", 5);
        assert_eq!(result, "rome");
    }

    #[test]
    fn slugify_filters_planner_extra_stop_words() {
        // "return", "exactly", "using", "use", "report", "fact" are planner extras
        let result = slugify("return exactly using the report fact", 5);
        assert_eq!(result, "mission");
    }

    #[test]
    fn slugify_unique_fallback_non_empty() {
        let result = slugify_unique_fallback("", 5);
        // Should be an 8-char UUID prefix, not "mission"
        assert_ne!(result, "mission");
        assert_eq!(result.len(), 8);
    }
}
