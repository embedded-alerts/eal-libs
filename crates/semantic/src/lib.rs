//! Deterministic URL, scoring, identity, cooldown, and retry helpers.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt::{self, Write},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError(String);

impl SemanticError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SemanticError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedUrl {
    pub canonical: String,
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
}

pub fn canonicalize_url(input: &str) -> Result<NormalizedUrl, SemanticError> {
    let trimmed = input.trim();
    let (raw_scheme, remainder) = trimmed
        .split_once("://")
        .ok_or_else(|| SemanticError::new("URL must include an http or https scheme"))?;
    let scheme = raw_scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(SemanticError::new("URL scheme must be http or https"));
    }

    let authority_end = remainder
        .char_indices()
        .find_map(|(index, character)| matches!(character, '/' | '?' | '#').then_some(index))
        .unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let tail = &remainder[authority_end..];
    if authority.is_empty() {
        return Err(SemanticError::new("URL host must not be empty"));
    }
    if authority.contains('@') {
        return Err(SemanticError::new("URL user information is not allowed"));
    }

    let (host, port) = parse_authority(authority)?;
    let without_fragment = tail.split('#').next().unwrap_or_default();
    let (raw_path, raw_query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_fragment, None),
    };
    let path = normalize_path(raw_path)?;
    let query = normalize_query(raw_query);
    let effective_port = match (scheme.as_str(), port) {
        ("http", Some(80)) | ("https", Some(443)) => None,
        _ => port,
    };
    let authority_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };

    let mut canonical = format!("{scheme}://{authority_host}");
    if let Some(value) = effective_port {
        write!(&mut canonical, ":{value}").expect("writing to a String cannot fail");
    }
    canonical.push_str(&path);
    if let Some(value) = query {
        canonical.push('?');
        canonical.push_str(&value);
    }

    Ok(NormalizedUrl {
        canonical,
        scheme,
        host,
        port: effective_port,
    })
}

fn parse_authority(authority: &str) -> Result<(String, Option<u16>), SemanticError> {
    let (raw_host, raw_port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed
            .find(']')
            .ok_or_else(|| SemanticError::new("bracketed IPv6 host is missing closing bracket"))?;
        let host = &bracketed[..close];
        let suffix = &bracketed[close + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .ok_or_else(|| SemanticError::new("invalid characters after IPv6 host"))?,
            )
        };
        (host, port)
    } else {
        let colon_count = authority.bytes().filter(|byte| *byte == b':').count();
        if colon_count > 1 {
            return Err(SemanticError::new("IPv6 hosts must use brackets"));
        }
        if colon_count == 1 {
            let (host, port) = authority
                .rsplit_once(':')
                .ok_or_else(|| SemanticError::new("invalid URL authority"))?;
            (host, Some(port))
        } else {
            (authority, None)
        }
    };

    let host = raw_host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(SemanticError::new("URL host must not be empty"));
    }
    if host
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '/' | '?' | '#'))
    {
        return Err(SemanticError::new("URL host contains invalid characters"));
    }

    let port = match raw_port {
        Some(value) => {
            let parsed = value
                .parse::<u16>()
                .map_err(|_| SemanticError::new("URL port must be an integer from 1 to 65535"))?;
            if parsed == 0 {
                return Err(SemanticError::new(
                    "URL port must be an integer from 1 to 65535",
                ));
            }
            Some(parsed)
        }
        None => None,
    };

    Ok((host, port))
}

fn normalize_path(raw_path: &str) -> Result<String, SemanticError> {
    if raw_path.is_empty() {
        return Ok("/".into());
    }
    if !raw_path.starts_with('/') {
        return Err(SemanticError::new("URL path must start with a slash"));
    }

    let preserve_trailing_slash = raw_path.ends_with('/');
    let mut segments = Vec::new();
    for segment in raw_path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }

    let mut path = String::from("/");
    path.push_str(&segments.join("/"));
    if preserve_trailing_slash && path != "/" {
        path.push('/');
    }
    Ok(path)
}

fn normalize_query(raw_query: Option<&str>) -> Option<String> {
    let mut pairs = raw_query?
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (name, value, had_equals) = match pair.split_once('=') {
                Some((name, value)) => (name, value, true),
                None => (pair, "", false),
            };
            (!is_tracking_parameter(name)).then_some((name, value, had_equals))
        })
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    if pairs.is_empty() {
        return None;
    }

    Some(
        pairs
            .into_iter()
            .map(|(name, value, had_equals)| {
                if had_equals {
                    format!("{name}={value}")
                } else {
                    name.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("&"),
    )
}

fn is_tracking_parameter(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized.starts_with("utm_")
        || normalized.starts_with("mc_")
        || matches!(
            normalized.as_str(),
            "gclid" | "dclid" | "fbclid" | "msclkid"
        )
}

pub fn normalize_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut needs_space = false;
    for character in input.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if needs_space && !output.is_empty() {
                output.push(' ');
            }
            output.push(character);
            needs_space = false;
        } else {
            needs_space = true;
        }
    }
    output
}

fn contains_phrase(normalized_document: &str, phrase: &str) -> bool {
    let normalized_phrase = normalize_text(phrase);
    if normalized_phrase.is_empty() {
        return false;
    }
    let padded_document = format!(" {normalized_document} ");
    let padded_phrase = format!(" {normalized_phrase} ");
    padded_document.contains(&padded_phrase)
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalEvaluation {
    pub score: f32,
    pub eligible: bool,
    pub matched_query_terms: usize,
    pub query_terms: usize,
    pub missing_required_terms: Vec<String>,
    pub excluded_term_hits: Vec<String>,
}

pub fn lexical_score(
    query: &str,
    document: &str,
    required_terms: &[String],
    excluded_terms: &[String],
) -> LexicalEvaluation {
    let normalized_query = normalize_text(query);
    let normalized_document = normalize_text(document);
    let query_tokens = normalized_query.split_whitespace().collect::<BTreeSet<_>>();
    let document_tokens = normalized_document
        .split_whitespace()
        .collect::<BTreeSet<_>>();
    let matched_query_terms = query_tokens.intersection(&document_tokens).count();
    let query_terms = query_tokens.len();
    let score = if query_terms == 0 {
        0.0
    } else {
        matched_query_terms as f32 / query_terms as f32
    };
    let missing_required_terms = required_terms
        .iter()
        .filter(|term| !contains_phrase(&normalized_document, term))
        .cloned()
        .collect::<Vec<_>>();
    let excluded_term_hits = excluded_terms
        .iter()
        .filter(|term| contains_phrase(&normalized_document, term))
        .cloned()
        .collect::<Vec<_>>();
    let eligible = missing_required_terms.is_empty() && excluded_term_hits.is_empty();

    LexicalEvaluation {
        score,
        eligible,
        matched_query_terms,
        query_terms,
        missing_required_terms,
        excluded_term_hits,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingNormalization {
    None,
    L2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpaceKey {
    pub provider: String,
    pub model: String,
    pub model_version: String,
    pub dimensions: usize,
    pub normalization: EmbeddingNormalization,
}

impl EmbeddingSpaceKey {
    pub fn validate(&self) -> Result<(), SemanticError> {
        for (field, value) in [
            ("provider", self.provider.as_str()),
            ("model", self.model.as_str()),
            ("model_version", self.model_version.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(SemanticError::new(format!(
                    "embedding space {field} must not be empty"
                )));
            }
        }
        if self.dimensions == 0 || self.dimensions > 32_768 {
            return Err(SemanticError::new(
                "embedding dimensions must be between 1 and 32768",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EmbeddingRef<'a> {
    pub space: &'a EmbeddingSpaceKey,
    pub values: &'a [f32],
}

pub fn cosine_similarity(
    query: EmbeddingRef<'_>,
    document: EmbeddingRef<'_>,
) -> Result<f32, SemanticError> {
    query.space.validate()?;
    document.space.validate()?;
    if query.space != document.space {
        return Err(SemanticError::new(
            "embedding spaces must match exactly before comparison",
        ));
    }
    if query.values.len() != query.space.dimensions
        || document.values.len() != document.space.dimensions
    {
        return Err(SemanticError::new(
            "embedding vector length does not match its declared space",
        ));
    }
    if query
        .values
        .iter()
        .chain(document.values)
        .any(|value| !value.is_finite())
    {
        return Err(SemanticError::new(
            "embedding vectors must contain only finite values",
        ));
    }

    let mut dot_product = 0.0_f64;
    let mut query_norm = 0.0_f64;
    let mut document_norm = 0.0_f64;
    for (query_value, document_value) in query.values.iter().zip(document.values) {
        let query_value = f64::from(*query_value);
        let document_value = f64::from(*document_value);
        dot_product += query_value * document_value;
        query_norm += query_value * query_value;
        document_norm += document_value * document_value;
    }
    if query_norm <= f64::EPSILON || document_norm <= f64::EPSILON {
        return Err(SemanticError::new(
            "embedding vectors must have non-zero magnitude",
        ));
    }

    let cosine = dot_product / (query_norm.sqrt() * document_norm.sqrt());
    Ok(cosine.clamp(0.0, 1.0) as f32)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreWeights {
    pub semantic: f32,
    pub lexical: f32,
}

impl ScoreWeights {
    pub fn validate(&self) -> Result<(), SemanticError> {
        if !self.semantic.is_finite() || !self.lexical.is_finite() {
            return Err(SemanticError::new("score weights must be finite"));
        }
        if !(0.0..=1.0).contains(&self.semantic) || !(0.0..=1.0).contains(&self.lexical) {
            return Err(SemanticError::new("score weights must be between 0 and 1"));
        }
        if (self.semantic + self.lexical - 1.0).abs() > 0.000_1 {
            return Err(SemanticError::new(
                "semantic and lexical weights must sum to 1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchRule {
    pub query_text: String,
    pub threshold: f32,
    pub weights: ScoreWeights,
    pub required_terms: Vec<String>,
    pub excluded_terms: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchRejection {
    MissingRequiredTerms,
    ExcludedTerms,
    BelowThreshold,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchEvaluation {
    pub matched: bool,
    pub lexical: LexicalEvaluation,
    pub semantic_score: Option<f32>,
    pub total_score: f32,
    pub threshold: f32,
    pub rejection: Option<MatchRejection>,
}

pub fn evaluate_match(
    rule: &MatchRule,
    document_text: &str,
    query_embedding: Option<EmbeddingRef<'_>>,
    document_embedding: Option<EmbeddingRef<'_>>,
) -> Result<MatchEvaluation, SemanticError> {
    rule.weights.validate()?;
    if !rule.threshold.is_finite() || !(0.0..=1.0).contains(&rule.threshold) {
        return Err(SemanticError::new(
            "match threshold must be finite and between 0 and 1",
        ));
    }

    let lexical = lexical_score(
        &rule.query_text,
        document_text,
        &rule.required_terms,
        &rule.excluded_terms,
    );
    let semantic_score = match (query_embedding, document_embedding) {
        (Some(query), Some(document)) => Some(cosine_similarity(query, document)?),
        (None, None) if rule.weights.semantic.abs() <= f32::EPSILON => None,
        (None, None) => {
            return Err(SemanticError::new(
                "semantic weight must be zero when embeddings are absent",
            ));
        }
        _ => {
            return Err(SemanticError::new(
                "query and document embeddings must either both be present or both be absent",
            ));
        }
    };
    let total_score = semantic_score.unwrap_or_default() * rule.weights.semantic
        + lexical.score * rule.weights.lexical;
    let rejection = if !lexical.missing_required_terms.is_empty() {
        Some(MatchRejection::MissingRequiredTerms)
    } else if !lexical.excluded_term_hits.is_empty() {
        Some(MatchRejection::ExcludedTerms)
    } else if total_score < rule.threshold {
        Some(MatchRejection::BelowThreshold)
    } else {
        None
    };

    Ok(MatchEvaluation {
        matched: rejection.is_none(),
        lexical,
        semantic_score,
        total_score,
        threshold: rule.threshold,
        rejection,
    })
}

pub fn match_identity(
    tenant_id: &str,
    rule_revision_id: &str,
    source_revision_id: &str,
    embedding_space_id: Option<&str>,
    content_sha256: &str,
) -> Result<String, SemanticError> {
    let components = [tenant_id, rule_revision_id, source_revision_id];
    if components
        .iter()
        .any(|value| value.trim().is_empty() || value.contains('|'))
    {
        return Err(SemanticError::new(
            "match identity components must be non-empty and must not contain pipes",
        ));
    }
    if embedding_space_id.is_some_and(|value| value.trim().is_empty() || value.contains('|')) {
        return Err(SemanticError::new(
            "embedding space identity must be non-empty and must not contain pipes",
        ));
    }
    if content_sha256.len() != 64 || !content_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SemanticError::new(
            "content hash must be a SHA-256 hexadecimal digest",
        ));
    }

    let payload = format!(
        "{tenant_id}|{rule_revision_id}|{source_revision_id}|{}|{}",
        embedding_space_id.unwrap_or("lexical-only"),
        content_sha256.to_ascii_lowercase()
    );
    Ok(sha256_hex(payload.as_bytes()))
}

pub fn delivery_idempotency_key(
    match_id: &str,
    delivery_target_id: &str,
    attempt_number: u32,
) -> Result<String, SemanticError> {
    if attempt_number == 0 {
        return Err(SemanticError::new("attempt number must be at least 1"));
    }
    for value in [match_id, delivery_target_id] {
        if value.trim().is_empty() || value.contains('|') {
            return Err(SemanticError::new(
                "delivery identity components must be non-empty and must not contain pipes",
            ));
        }
    }
    Ok(sha256_hex(
        format!("delivery|{match_id}|{delivery_target_id}|{attempt_number}").as_bytes(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CooldownDecision {
    NotifyNow,
    SuppressedUntil(u64),
}

pub fn cooldown_decision(
    now_epoch_seconds: u64,
    last_delivery_epoch_seconds: Option<u64>,
    cooldown_seconds: u64,
) -> CooldownDecision {
    let Some(last_delivery) = last_delivery_epoch_seconds else {
        return CooldownDecision::NotifyNow;
    };
    let next_allowed = last_delivery.saturating_add(cooldown_seconds);
    if now_epoch_seconds >= next_allowed {
        CooldownDecision::NotifyNow
    } else {
        CooldownDecision::SuppressedUntil(next_allowed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub base_seconds: u64,
    pub max_seconds: u64,
    pub max_attempts: u32,
    pub jitter_percent: u8,
}

impl RetryPolicy {
    pub fn validate(&self) -> Result<(), SemanticError> {
        if self.base_seconds == 0 {
            return Err(SemanticError::new("retry base seconds must be positive"));
        }
        if self.max_seconds < self.base_seconds {
            return Err(SemanticError::new(
                "retry max seconds must be at least the base seconds",
            ));
        }
        if self.max_attempts == 0 {
            return Err(SemanticError::new("retry max attempts must be positive"));
        }
        if self.jitter_percent > 50 {
            return Err(SemanticError::new(
                "retry jitter percent must not exceed 50",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    RetryAfter(u64),
    DeadLetter,
}

pub fn retry_decision(
    policy: RetryPolicy,
    idempotency_key: &str,
    failed_attempt: u32,
) -> Result<RetryDecision, SemanticError> {
    policy.validate()?;
    if idempotency_key.trim().is_empty() {
        return Err(SemanticError::new("idempotency key must not be empty"));
    }
    if failed_attempt == 0 {
        return Err(SemanticError::new("failed attempt must be at least 1"));
    }
    if failed_attempt >= policy.max_attempts {
        return Ok(RetryDecision::DeadLetter);
    }

    let exponent = failed_attempt.saturating_sub(1).min(63);
    let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
    let delay = policy
        .base_seconds
        .saturating_mul(multiplier)
        .min(policy.max_seconds);
    let jitter_window = delay.saturating_mul(u64::from(policy.jitter_percent)) / 100;
    if jitter_window == 0 {
        return Ok(RetryDecision::RetryAfter(delay));
    }

    let digest = sha256(idempotency_key.as_bytes());
    let sample = u64::from_be_bytes(digest[..8].try_into().expect("slice length is fixed"));
    let range = jitter_window.saturating_mul(2).saturating_add(1);
    let sampled_offset = i128::from(sample % range) - i128::from(jitter_window);
    let jittered =
        (i128::from(delay) + sampled_offset).clamp(1, i128::from(policy.max_seconds)) as u64;
    Ok(RetryDecision::RetryAfter(jittered))
}

pub fn sha256_hex(input: &[u8]) -> String {
    let digest = sha256(input);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL_STATE: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const ROUND_CONSTANTS: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_length = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut state = INITIAL_STATE;
    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0_u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes(
                chunk[offset..offset + 4]
                    .try_into()
                    .expect("SHA-256 chunks contain complete words"),
            );
        }
        let mut index = 16;
        while index < 64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
            index += 1;
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for (&constant, &word) in ROUND_CONSTANTS.iter().zip(schedule.iter()) {
            let upper_sigma = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary_one = h
                .wrapping_add(upper_sigma)
                .wrapping_add(choice)
                .wrapping_add(constant)
                .wrapping_add(word);
            let lower_sigma = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary_two = lower_sigma.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary_one);
            d = c;
            c = b;
            b = a;
            a = temporary_one.wrapping_add(temporary_two);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut digest = [0_u8; 32];
    for (index, word) in state.iter().enumerate() {
        let offset = index * 4;
        digest[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(model_version: &str, dimensions: usize) -> EmbeddingSpaceKey {
        EmbeddingSpaceKey {
            provider: "local".into(),
            model: "mini".into(),
            model_version: model_version.into(),
            dimensions,
            normalization: EmbeddingNormalization::L2,
        }
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn canonicalizes_hosts_paths_and_tracking_parameters() {
        let normalized =
            canonicalize_url("HTTPS://Example.COM:443/a/./b/../c/?utm_source=x&z=2&a=1#fragment")
                .unwrap();
        assert_eq!(normalized.canonical, "https://example.com/a/c/?a=1&z=2");
        assert_eq!(normalized.host, "example.com");
        assert_eq!(normalized.port, None);
    }

    #[test]
    fn rejects_user_information() {
        assert!(canonicalize_url("https://user:pass@example.com/").is_err());
    }

    #[test]
    fn lexical_matching_enforces_required_and_excluded_phrases() {
        let required = vec!["rust service".into()];
        let excluded = vec!["next js".into()];
        let evaluation = lexical_score(
            "rust alerts",
            "A Rust service for semantic alerts, without a JavaScript framework.",
            &required,
            &excluded,
        );
        assert!(evaluation.eligible);
        assert!((evaluation.score - 1.0).abs() <= f32::EPSILON);

        let blocked = lexical_score(
            "rust alerts",
            "A Rust service for semantic alerts built with Next.js.",
            &required,
            &excluded,
        );
        assert!(!blocked.eligible);
        assert_eq!(blocked.excluded_term_hits, vec!["next js"]);
    }

    #[test]
    fn rejects_cross_model_comparisons() {
        let query_space = space("1", 2);
        let document_space = space("2", 2);
        let error = cosine_similarity(
            EmbeddingRef {
                space: &query_space,
                values: &[1.0, 0.0],
            },
            EmbeddingRef {
                space: &document_space,
                values: &[1.0, 0.0],
            },
        )
        .unwrap_err();
        assert!(error.message().contains("must match exactly"));
    }

    #[test]
    fn combines_semantic_and_lexical_scores_deterministically() {
        let embedding_space = space("1", 2);
        let rule = MatchRule {
            query_text: "rust alerts".into(),
            threshold: 0.8,
            weights: ScoreWeights {
                semantic: 0.75,
                lexical: 0.25,
            },
            required_terms: vec!["rust".into()],
            excluded_terms: vec![],
        };
        let evaluation = evaluate_match(
            &rule,
            "Rust alerts for new pages",
            Some(EmbeddingRef {
                space: &embedding_space,
                values: &[1.0, 0.0],
            }),
            Some(EmbeddingRef {
                space: &embedding_space,
                values: &[0.9, 0.1],
            }),
        )
        .unwrap();
        assert!(evaluation.matched);
        assert!(evaluation.total_score > 0.99);
    }

    #[test]
    fn match_identity_is_stable_and_model_scoped() {
        let first = match_identity(
            "tenant",
            "rule-revision",
            "source-revision",
            Some("space-a"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        let repeated = match_identity(
            "tenant",
            "rule-revision",
            "source-revision",
            Some("space-a"),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        let other_space = match_identity(
            "tenant",
            "rule-revision",
            "source-revision",
            Some("space-b"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap();
        assert_eq!(first, repeated);
        assert_ne!(first, other_space);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn cooldown_suppresses_delivery_without_discarding_the_match() {
        assert_eq!(
            cooldown_decision(1_100, Some(1_000), 300),
            CooldownDecision::SuppressedUntil(1_300)
        );
        assert_eq!(
            cooldown_decision(1_300, Some(1_000), 300),
            CooldownDecision::NotifyNow
        );
    }

    #[test]
    fn retry_policy_is_capped_and_eventually_dead_letters() {
        let policy = RetryPolicy {
            base_seconds: 10,
            max_seconds: 100,
            max_attempts: 4,
            jitter_percent: 0,
        };
        assert_eq!(
            retry_decision(policy, "delivery-key", 1).unwrap(),
            RetryDecision::RetryAfter(10)
        );
        assert_eq!(
            retry_decision(policy, "delivery-key", 3).unwrap(),
            RetryDecision::RetryAfter(40)
        );
        assert_eq!(
            retry_decision(policy, "delivery-key", 4).unwrap(),
            RetryDecision::DeadLetter
        );
    }
}
