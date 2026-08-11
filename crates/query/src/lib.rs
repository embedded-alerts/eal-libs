use eal_semantic_contracts::{
    EmbeddingInput, EmbeddingInputKind, QueryTextViews, SemanticContractError,
};
use std::{collections::BTreeSet, error::Error, fmt};

const MAX_QUERY_CHARS: usize = 700;
const MAX_KEYWORDS: usize = 24;
const MAX_ENTITIES: usize = 24;
const STOPWORDS: &str = concat!(
    "a about after again all also am an and any are as at be because been before being between ",
    "both but by can could did do does doing down during each few for from further had has have ",
    "having he her here hers herself him himself his how i if in into is it its itself just me ",
    "more most my myself no nor not now of off on once only or other our ours ourselves out over ",
    "own same she should so some such than that the their theirs them themselves then there these ",
    "they this those through to too under until up very was we were what when where which while who ",
    "whom why will with would you your yours yourself yourselves",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryAnalysisError {
    pub code: &'static str,
    pub message: String,
}

impl QueryAnalysisError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for QueryAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for QueryAnalysisError {}

pub fn analyze_query(query: &str) -> Result<QueryTextViews, QueryAnalysisError> {
    let query_text = collapse_whitespace(query);
    let character_count = query_text.chars().count();
    if !(3..=MAX_QUERY_CHARS).contains(&character_count) {
        return Err(QueryAnalysisError::new(
            "invalid_query_text",
            format!("query must contain 3 to {MAX_QUERY_CHARS} characters"),
        ));
    }

    let keywords = extract_keywords(&query_text, MAX_KEYWORDS);
    let entities = extract_entities(&query_text, MAX_ENTITIES);
    let mut embedding_inputs = vec![EmbeddingInput {
        kind: EmbeddingInputKind::Query,
        ordinal: 0,
        text: query_text.clone(),
        weight: 1.25,
    }];

    for group in entities.chunks(6) {
        embedding_inputs.push(EmbeddingInput {
            kind: EmbeddingInputKind::Entity,
            ordinal: 0,
            text: group.join("; "),
            weight: 1.0,
        });
    }
    for group in keywords.chunks(8) {
        embedding_inputs.push(EmbeddingInput {
            kind: EmbeddingInputKind::Keyword,
            ordinal: 0,
            text: group.join(", "),
            weight: 0.85,
        });
    }
    for (ordinal, input) in embedding_inputs.iter_mut().enumerate() {
        input.ordinal = u16::try_from(ordinal).map_err(|_| {
            QueryAnalysisError::new("query_too_complex", "query produced too many embedding views")
        })?;
    }

    let views = QueryTextViews {
        query_text,
        keywords,
        entities,
        embedding_inputs,
    };
    views
        .validate()
        .map_err(QueryAnalysisError::from_contract)?;
    Ok(views)
}

pub fn combined_query_text(query: &str) -> Result<String, QueryAnalysisError> {
    let views = analyze_query(query)?;
    Ok(combine_inputs(&views.embedding_inputs))
}

fn combine_inputs(inputs: &[EmbeddingInput]) -> String {
    inputs
        .iter()
        .map(|input| format!("{}: {}", input.kind.wire_name(), input.text))
        .collect::<Vec<_>>()
        .join("\n")
}

impl QueryAnalysisError {
    fn from_contract(error: SemanticContractError) -> Self {
        Self::new(error.code, error.message)
    }
}

fn extract_keywords(text: &str, limit: usize) -> Vec<String> {
    let mut keywords = BTreeSet::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || (character == '\'' && !current.is_empty()) {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            insert_keyword(&mut keywords, std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        insert_keyword(&mut keywords, current);
    }
    keywords.into_iter().take(limit).collect()
}

fn insert_keyword(keywords: &mut BTreeSet<String>, token: String) {
    if token.chars().count() >= 3
        && !token.chars().all(|character| character.is_numeric())
        && !is_stopword(&token)
    {
        keywords.insert(token);
    }
}

fn extract_entities(text: &str, limit: usize) -> Vec<String> {
    let mut entities = BTreeSet::new();
    let mut phrase = Vec::new();
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|character: char| {
            !character.is_alphanumeric() && character != '-' && character != '\''
        });
        if is_entity_token(token) {
            phrase.push(token.to_owned());
            if phrase.len() == 5 {
                flush_entity(&mut phrase, &mut entities);
            }
        } else {
            flush_entity(&mut phrase, &mut entities);
        }
    }
    flush_entity(&mut phrase, &mut entities);
    entities.into_iter().take(limit).collect()
}

fn flush_entity(phrase: &mut Vec<String>, entities: &mut BTreeSet<String>) {
    if phrase.is_empty() {
        return;
    }
    let candidate = phrase.join(" ");
    if candidate.chars().count() >= 3 && !is_common_sentence_lead(&candidate) {
        entities.insert(candidate);
    }
    phrase.clear();
}

fn is_entity_token(token: &str) -> bool {
    if token.chars().count() < 2 || token.chars().all(|character| character.is_numeric()) {
        return false;
    }
    let Some(first) = token.chars().next() else {
        return false;
    };
    if !first.is_uppercase() {
        return false;
    }
    let letters = token
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect::<Vec<_>>();
    let uppercase = letters
        .iter()
        .filter(|character| character.is_uppercase())
        .count();
    uppercase == 1 || (uppercase == letters.len() && letters.len() <= 8)
}

fn is_common_sentence_lead(candidate: &str) -> bool {
    matches!(
        candidate,
        "A" | "An"
            | "And"
            | "But"
            | "For"
            | "How"
            | "In"
            | "It"
            | "Notify"
            | "On"
            | "Or"
            | "Show"
            | "Tell"
            | "The"
            | "This"
            | "To"
            | "We"
            | "What"
            | "When"
            | "Where"
            | "Why"
            | "You"
    )
}

fn is_stopword(token: &str) -> bool {
    STOPWORDS
        .split_ascii_whitespace()
        .any(|stopword| stopword == token)
}

fn collapse_whitespace(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut pending_space = false;
    for character in input.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
            }
            output.push(character);
            pending_space = false;
        }
    }
    output.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_query_is_preserved_as_the_strongest_view() {
        let query = "Notify me when Acme launches renewable energy tools in Colombia.";
        let views = analyze_query(query).unwrap();
        assert_eq!(views.embedding_inputs[0].kind, EmbeddingInputKind::Query);
        assert_eq!(views.embedding_inputs[0].text, query);
        assert_eq!(views.embedding_inputs[0].weight, 1.25);
    }

    #[test]
    fn keywords_and_proper_nouns_are_companion_views() {
        let views = analyze_query(
            "Notify me when Acme Corporation launches renewable energy tools in Colombia.",
        )
        .unwrap();
        assert!(views.keywords.iter().any(|keyword| keyword == "renewable"));
        assert!(
            views
                .entities
                .iter()
                .any(|entity| entity.contains("Acme Corporation"))
        );
        assert!(
            views
                .embedding_inputs
                .iter()
                .any(|input| input.kind == EmbeddingInputKind::Keyword)
        );
        assert!(
            views
                .embedding_inputs
                .iter()
                .any(|input| input.kind == EmbeddingInputKind::Entity)
        );
    }

    #[test]
    fn query_validation_is_bounded() {
        assert!(analyze_query("no").is_err());
        assert!(analyze_query(&"x".repeat(MAX_QUERY_CHARS + 1)).is_err());
    }

    #[test]
    fn combined_query_text_is_labeled() {
        let text = combined_query_text("Acme renewable energy launches").unwrap();
        assert!(text.starts_with("query: Acme renewable energy launches"));
        assert!(text.contains("keyword:"));
    }
}
