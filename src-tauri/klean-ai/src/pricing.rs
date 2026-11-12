//! What a session costs.
//!
//! Prices are per million tokens; the result is in **dollars**, which is what the
//! modal shows (four decimals, because a single turn is often a fraction of one).
//!
//! Prices are the published per-million-token rates, in USD, matched against the
//! model name by prefix so that dated variants (`claude-sonnet-4-20250514`) and
//! gateway suffixes still land on the right row. A model nobody has priced gets no
//! cost at all, which the modal shows as tokens only — better than inventing a
//! number.

use crate::model::{ProviderConfig, Usage};

/// Input and output price, in USD per million tokens.
#[derive(Debug, Clone, Copy)]
struct Price {
    /// Matched against the lowercased model name, as a prefix.
    model: &'static str,
    input: f64,
    output: f64,
}

const PRICES: &[Price] = &[
    // DeepSeek
    Price { model: "deepseek-reasoner", input: 0.55, output: 2.19 },
    Price { model: "deepseek-chat", input: 0.27, output: 1.10 },
    Price { model: "deepseek", input: 0.27, output: 1.10 },
    // Anthropic
    Price { model: "claude-opus", input: 15.00, output: 75.00 },
    Price { model: "claude-sonnet", input: 3.00, output: 15.00 },
    Price { model: "claude-haiku", input: 0.80, output: 4.00 },
    Price { model: "claude-3-5-haiku", input: 0.80, output: 4.00 },
    Price { model: "claude", input: 3.00, output: 15.00 },
    // OpenAI
    Price { model: "gpt-4o-mini", input: 0.15, output: 0.60 },
    Price { model: "gpt-4o", input: 2.50, output: 10.00 },
    Price { model: "gpt-4.1-mini", input: 0.40, output: 1.60 },
    Price { model: "gpt-4.1", input: 2.00, output: 8.00 },
    Price { model: "o4-mini", input: 1.10, output: 4.40 },
    Price { model: "o3", input: 2.00, output: 8.00 },
    Price { model: "gpt", input: 2.50, output: 10.00 },
];

/// The price row for a model name, if there is one.
fn price_of(model: &str) -> Option<Price> {
    let name = model.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    PRICES
        .iter()
        .filter(|price| name.starts_with(price.model))
        // The longest match wins: `gpt-4o-mini` must not read as `gpt-4o`.
        .max_by_key(|price| price.model.len())
        .copied()
}

/// What `usage` cost, in dollars, for the model this config names.
pub fn dollars(config: &ProviderConfig, usage: Usage) -> Option<f64> {
    let price = price_of(&config.model)?;
    if usage.is_empty() {
        return Some(0.0);
    }
    let input = usage.input_tokens as f64 / 1_000_000.0 * price.input;
    let output = usage.output_tokens as f64 / 1_000_000.0 * price.output;
    Some(input + output)
}

/// The per-million rates, for the settings panel and for tests.
pub fn rates(model: &str) -> Option<(f64, f64)> {
    price_of(model).map(|price| (price.input, price.output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;

    fn config(model: &str, kind: ProviderKind) -> ProviderConfig {
        ProviderConfig {
            kind,
            model: model.to_string(),
            ..ProviderConfig::default()
        }
    }

    #[test]
    fn a_million_tokens_of_deepseek_costs_its_rate() {
        let usage = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000 };
        let dollars = dollars(&config("deepseek-chat", ProviderKind::DeepSeek), usage).unwrap();
        // 0.27 + 1.10 dollars.
        assert!((dollars - 1.37).abs() < 0.0001, "{dollars}");
    }

    #[test]
    fn the_specific_model_wins() {
        let usage = Usage { input_tokens: 1_000_000, output_tokens: 0 };
        let mini = dollars(&config("gpt-4o-mini", ProviderKind::OpenAi), usage).unwrap();
        let full = dollars(&config("gpt-4o", ProviderKind::OpenAi), usage).unwrap();
        assert!(mini < full, "{mini} vs {full}");
        assert!((mini - 0.15).abs() < 0.0001, "{mini}");
    }

    #[test]
    fn dated_variants_still_match() {
        let dollars = dollars(
            &config("claude-sonnet-4-20250514", ProviderKind::Claude),
            Usage { input_tokens: 1_000_000, output_tokens: 0 },
        )
        .unwrap();
        assert!((dollars - 3.0).abs() < 0.0001, "{dollars}");
    }

    #[test]
    fn an_unpriced_model_has_no_cost() {
        assert!(dollars(&config("mystery-model", ProviderKind::OpenAi), Usage::default()).is_none());
        assert!(rates("mystery-model").is_none());
    }

    #[test]
    fn no_tokens_cost_nothing() {
        let dollars = dollars(&config("deepseek-chat", ProviderKind::DeepSeek), Usage::default());
        assert_eq!(dollars, Some(0.0));
    }
}
