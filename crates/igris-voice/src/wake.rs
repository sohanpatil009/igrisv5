//! Wake-word matching on transcripts: exact phrase plus a Levenshtein
//! tolerance of 2 per word ("arise" ~ "arice"). Acoustic wake models plug
//! in front later; the phrase, threshold, and matcher API are stable.

#[derive(Debug, Clone)]
pub struct WakeMatcher {
    pub phrase: String,
    pub max_distance: usize,
}

impl WakeMatcher {
    pub fn arise() -> Self {
        Self {
            phrase: "arise".into(),
            max_distance: 2,
        }
    }

    pub fn new(phrase: &str) -> Self {
        Self {
            phrase: phrase.to_lowercase(),
            max_distance: 2,
        }
    }

    pub fn matches(&self, transcript: &str) -> bool {
        let t = transcript.to_lowercase();
        if t.contains(&self.phrase) {
            return true;
        }
        t.split_whitespace()
            .any(|w| levenshtein(w, &self.phrase) <= self.max_distance)
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            cur.push((prev[j + 1] + 1).min((cur[j] + 1).min(prev[j] + usize::from(ca != cb))));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_fuzzy_match() {
        let w = WakeMatcher::arise();
        assert!(w.matches("Arise, what is my battery?"));
        assert!(w.matches("arice tell me the time")); // distance 1
        assert!(!w.matches("what time is it"));
        assert!(!w.matches(""));
    }

    #[test]
    fn far_words_rejected() {
        let w = WakeMatcher::arise();
        assert!(!w.matches("around the clock"));
        assert!(w.matches("arise and shine"));
    }
}
